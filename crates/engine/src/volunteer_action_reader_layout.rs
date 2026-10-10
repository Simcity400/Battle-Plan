//! Volunteer-action reader metadata. Country references are resolved
//! before obtaining volunteer-host lists.

/// Contains-check resolving receiver +0x14 and checking host membership against +0x1c.
pub const VOLUNTEER_HOST_CONTAINS_READER_RVA: usize = 0x0112_5860;
/// Relation query resolving +0x14/+0x1c and testing counterpart relation slots.
pub const VOLUNTEER_HOST_RELATION_READER_RVA: usize = 0x0112_5c40;
