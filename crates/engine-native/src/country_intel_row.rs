//! Four-lane entry in the country intel first vector; concrete entry type is
//! unresolved. Recording retains signed i64 amounts without normalization.
//! Serialization writes one block per entry and emits each nonzero lane as
//! fixed5. Saved decimals divide raw amounts by 100,000; negative lanes are
//! ordinary. Omission means exact zero, and an all-zero entry still produces
//! an empty block.

/// Country-intel entry stride.
pub const STRIDE: usize = 0x20;
/// Entry civilian amount (native i64).
pub const CIVILIAN: usize = 0x00;
/// Entry army amount (native i64).
pub const ARMY: usize = 0x08;
/// Entry navy amount (native i64).
pub const NAVY: usize = 0x10;
/// Air amount, signed i64, normalized for display with the runtime air maximum.
pub const AIR: usize = 0x18;

/// Record serializer emitting conditional lane keys in this file's order.
pub const WRITER_RVA: usize = 0x0109_aea0;
/// Lane denominator used by the signed fixed5 formatter.
pub const LANE_DENOMINATOR: i64 = 100_000;

/// Target-indexed record. Fog reads army and air; naval-base port detail
/// reads navy; raid target intel selects a lane from the building intel kind.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// Raw reference of the country whose CCountryIntel holds this record.
    pub owner_country: u32,
    /// Vector index is the target's canonical country-link slot. Resolve the
    /// raw country reference through the country-link table before indexing;
    /// all accessors use the resulting slot.
    pub target_index: u32,
    pub army: i64,
    pub air: i64,
    pub civilian: i64,
    pub navy: i64,
}

impl Row {
    /// Decodes the recorded lanes of one native 0x20-byte entry; every bit
    /// pattern is a valid i64.
    pub fn from_bytes(owner_country: u32, target_index: u32, bytes: &[u8; STRIDE]) -> Self {
        let lane =
            |offset: usize| i64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap());
        Self {
            owner_country,
            target_index,
            army: lane(ARMY),
            air: lane(AIR),
            civilian: lane(CIVILIAN),
            navy: lane(NAVY),
        }
    }

    /// Whether every recorded lane is zero.
    pub fn is_zero(&self) -> bool {
        self.army == 0 && self.air == 0 && self.civilian == 0 && self.navy == 0
    }
}
