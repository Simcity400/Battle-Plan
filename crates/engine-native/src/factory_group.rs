//! One factory group embedded in CProductionStatus: military, naval, civilian and rocket
//! production, 0x60 bytes each. The recorder retains the first three. The fourth counts positive
//! `rocket_production`, not sites with `rocket_launch_capacity`.
//!
//! Reset clears every group, and recalculation rebuilds all four from controlled
//! states and one offsite pass. Each building contributes to every category
//! whose signed production flag is positive; modded types are classified by
//! their flags. Country modifiers, consumers and transfers affect the first
//! three groups. Assignment distributes military and naval factories; rocket
//! production uses its separate line domain.
//!
//! Groups are not serialized and are recalculated after load. Fixed5 fields
//! retain signed i64 values; game views divide by the scale and truncate to i32.

/// Group byte size.
pub const SIZE: usize = 0x60;

/// Counted building levels, i32, including owned, occupied and offsite levels.
/// Recalculation sums signed short levels. Controlled-factory queries sum the
/// first three stored counts with i32 wrapping rather than using fixed5 totals.
pub const LEVELS: usize = 0x00;
/// Total, i64 fixed5. Building levels use the nonnegative state-output factor;
/// offsite buildings use the unit factor. Recalculation then adds category
/// modifiers, subject and other-country contributions, plus civilian licence
/// and trade exports. Construction displays the truncated total.
pub const TOTAL: usize = 0x08;
/// Controlled-state and offsite contribution, i64 fixed5. Uses the same
/// level/output factor as TOTAL without later additions. The displayed
/// occupation contribution subtracts truncated OWNED, so offsite buildings
/// are included in that difference.
pub const CONTROLLED: usize = 0x10;
/// Owned part of CONTROLLED, i64 fixed5; includes states owned by the country.
pub const OWNED: usize = 0x18;
/// Damaged factories, i64 fixed5; applies the output factor to level minus
/// healthy level. Consumers subtract the truncated value.
pub const DAMAGED: usize = 0x20;
/// Assigned factories, i32. Military and naval groups use production lines;
/// civilian assignment uses construction consumers. Recalculation clears and
/// refills the corresponding counts; construction is reassigned hourly.
pub const USED: usize = 0x28;
/// Factories unavailable before assignment, i32; includes TO_OVERLORD and
/// TO_OTHERS, plus allocated civilian-consumer counts for the civilian group.
pub const UNAVAILABLE: usize = 0x40;
/// Factories received from subjects, i32; uses rounded shares capped at the full amount.
pub const FROM_SUBJECTS: usize = 0x44;
/// Factories given to the overlord, i32; rounded TOTAL share also counted in UNAVAILABLE.
pub const TO_OVERLORD: usize = 0x48;
/// Factories received from other countries, i32; sums the receiver-side relation shares.
pub const FROM_OTHERS: usize = 0x4c;
/// Factories given to other countries, i32; giver-side TOTAL shares also enter UNAVAILABLE.
pub const TO_OTHERS: usize = 0x50;
/// Civilian factories lent for licences, i32; sums the stored licence counts.
pub const LICENSE_EXPORTS: usize = 0x54;
/// Civilian factories assigned to exports, i32; sums exporter-side exchanges
/// using the factories last assigned by importers' trade consumers.
pub const TRADE_EXPORTS: usize = 0x58;
