//! NIndustrialOrganisation::COrganisation gameplay fields encountered by the old MIO UI walks.
//! The old history hook followed the wrong controller backlink; these offsets
//! require a resolved organisation object, not its COrganisationListItem UI owner.
//! Hourly consistency and save coverage remain unproved.

/// COrganisation engine CID type; collector offsets.rs INDUSTRIAL_ORGANISATION_CID_TYPE accepts type 79.
pub const REFERENCE_TYPE_ID: u32 = 79;
/// COrganisation self-reference pair (u32 type, u32 id); a729_mio_history_hook resolve_identity checks +0x08.
pub const REFERENCE: usize = 0x08;
/// COrganisation native i64 base for modifier 0x26d; a729_mio_bonus_hook fixed_dispositions reads +0xc8.
pub const MODIFIER_26D_BASE: usize = 0xc8;
/// COrganisation production-line entry array; a729_mio_history_hook production_candidates reads +0xd8.
pub const PRODUCTION_LINES: usize = 0xd8;
/// COrganisation production-line entry count i32; a729_mio_history_hook production_candidates reads +0xe4.
pub const PRODUCTION_LINES_COUNT: usize = 0xe4;
/// COrganisation history hash-table array; a729_mio_history_hook reads +0xf8 before traversing 0x40-byte entries.
pub const HISTORY: usize = 0xf8;
/// COrganisation history table last index i32; old history hook bounds traversal by last+overflow+1.
pub const HISTORY_LAST: usize = 0x104;
/// COrganisation history table overflow count u8; a729_mio_history_hook reads +0x108.
pub const HISTORY_OVERFLOW: usize = 0x108;
/// COrganisation bonus equipment database pointer; a729_mio_bonus_hook reads +0x110.
pub const BONUS_EQUIPMENT_DATABASE: usize = 0x110;
pub const AGGREGATE_MODIFIER_RVA: usize = 0x00da_5230;
/// COrganisation neutral fixed modifier value; a729_mio_bonus_hook passes 100000 for modifiers 0x26e/0x26f.
pub const MODIFIER_NEUTRAL: i64 = 100_000;
