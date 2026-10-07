//! Stored supply-footer operands shared by authenticated divisions and railway guns.
//!
//! `CUnitCounterItem::tooltip_builder@1418C00D0` appends `1418BDBA0` for the
//! low-supply child, after the source owner's shared-vision gate (`1406F3A90`).
//! Each member resolves to its `CSupplyConsumer` secondary base, `CUnit+0x10`,
//! and `14161CCE0` reads the amounts below. `14161D560` formats their aggregate;
//! it does not reconstruct them from supply nodes, weather or the stored ratio.
//!
//! Reset `14121BAD0` clears receipts, origins and all three demand pairs before
//! calling the consumer's consumption getter. Division getter `140C657B0`
//! writes the division pair; other consumer classes can populate other pairs.
//! State allocation (`141200B80`, `1412055C0`, `140EA0080`) and air allocation
//! (`141205840`, `14120F2B0`) add their amounts. `1419F7530` inserts or adds an
//! origin's requested/received amounts, preserving the native array order, and
//! `1419F77F0` adds total receipts, capped at the asked amount. Disruption is
//! reset by `141219350` and increased by air-strike application `140F612F0`.
//!
//! `CSupplyConsumer::Serialize@140EBB270` saves only positive disruption under
//! token 0x4CF2 (`disrupted_supply`); `ReadToken@140EB5150` restores its full
//! signed fixed5 word. An omitted save token proves only nonpositive disruption.
//! The other operands are unsaved caches and need a matched native-memory
//! witness. This reader neither invokes the game nor creates a new unit census.

use crate::{Memory, cpdx_array::Array, memory::RecordLayout, memory::RecordSources, read};

/// Primary CUnit offsets: the embedded CSupplyConsumer starts at +0x10.
pub const ASKED: usize = 0x38;
pub const RECEIVED: usize = 0x50;
pub const AIR_DROPS: usize = 0x58;
pub const ORIGINS: usize = 0x60;
pub const FROM_STATE: usize = 0x78;
pub const DIVISION_COUNT: usize = 0x80;
pub const DIVISION_DEMAND: usize = 0x88;
pub const AIRPLANE_COUNT: usize = 0x90;
pub const AIRPLANE_DEMAND: usize = 0x98;
pub const SHIP_COUNT: usize = 0xa0;
pub const SHIP_DEMAND: usize = 0xa8;
pub const DISRUPTION: usize = 0xb0;

/// Native distributed-supply records (`1419F7530`), not supply-node objects.
pub const ORIGIN_STRIDE: usize = 0x30;
pub const ORIGIN_REQUESTED: usize = 0x10;
pub const ORIGIN_RECEIVED: usize = 0x18;
pub const ORIGIN_PROVINCE: usize = 0x28;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Demand {
    pub count: i32,
    /// Raw signed fixed5 demand; do not derive it from the count.
    pub value: i64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Origin {
    pub province: i32,
    pub requested: i64,
    pub received: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inputs {
    pub asked: i64,
    pub received: i64,
    pub from_state: i64,
    pub air_drops: i64,
    /// Aggregation overwrites this operand for each member, rather than adding it.
    pub disruption: i64,
    pub division_demand: Demand,
    pub airplane_demand: Demand,
    pub ship_demand: Demand,
    /// Complete ordered prefix of the caller's origin arena, or the failure of
    /// this independent node fragment. Scalar footer lines remain available.
    pub origins_len: Result<usize, Failure>,
}

impl Default for Inputs {
    fn default() -> Self {
        Self {
            asked: 0,
            received: 0,
            from_state: 0,
            air_drops: 0,
            disruption: 0,
            division_demand: Demand::default(),
            airplane_demand: Demand::default(),
            ship_demand: Demand::default(),
            origins_len: Ok(0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    Unreadable,
    InvalidOrigins,
    OriginsCapacity { required: usize },
    OriginsChanged,
}

impl Failure {
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Unreadable => "unit supply inputs unreadable",
            Self::InvalidOrigins => "unit supply origin header invalid",
            Self::OriginsCapacity { .. } => "unit supply origin arena too small",
            Self::OriginsChanged => "unit supply origin header changed",
        }
    }
}

// Exact scalar spans only. The independent origin control is acquired below;
// its read failure must not invalidate these footer lines.
const INPUT_SPANS: [(usize, usize); 6] = [
    (ASKED, 8),
    (RECEIVED, 16),
    (FROM_STATE, 12),
    (DIVISION_DEMAND, 12),
    (AIRPLANE_DEMAND, 12),
    (SHIP_DEMAND, 16),
];
const INPUT_FIELDS: [crate::memory::RecordField; 6] = crate::memory::packed_fields(INPUT_SPANS);
const INPUT_BYTES: usize = crate::memory::packed_stride(INPUT_SPANS);
const INPUT_LAYOUT: RecordLayout<'static> = match RecordLayout::new(&INPUT_FIELDS, INPUT_BYTES) {
    Some(layout) => layout,
    None => panic!("valid supply input layout"),
};

const ORIGIN_FIELDS: [crate::memory::RecordField; 2] =
    crate::memory::packed_fields([(ORIGIN_REQUESTED, 16), (ORIGIN_PROVINCE, 4)]);
const ORIGIN_BYTES: usize = 20;
const ORIGIN_LAYOUT: RecordLayout<'static> = match RecordLayout::new(&ORIGIN_FIELDS, ORIGIN_BYTES) {
    Some(layout) => layout,
    None => panic!("valid supply origin layout"),
};

/// Read one already authenticated primary CUnit root. The caller owns both its
/// unit census and the reusable origin arena. A shortage returns the exact live
/// count without copying a misleading partial vector. Origin failures withhold
/// only the node fragment; scalar lines stay available. Partially written arena
/// bytes are unusable. Scalar-copy failure withholds this unit's scalar pack.
///
/// All entries are retained, including duplicate provinces and nonpositive
/// amounts: totals consume every entry, while the footer's displayed hub list
/// consumes only positive receipts. No native array size becomes an allocation.
pub fn capture(
    memory: &impl Memory,
    pointer: usize,
    origins: &mut [Origin],
) -> Result<Inputs, Failure> {
    let mut packed = [0; INPUT_BYTES];
    if pointer == 0
        || memory.copy_records(
            RecordSources::Pointers(&[pointer]),
            &INPUT_LAYOUT,
            &mut packed,
        ) != INPUT_FIELDS.len()
    {
        return Err(Failure::Unreadable);
    }
    let origins_len = capture_origins(memory, pointer, origins);
    let i64_at = |at| i64::from_le_bytes(packed[at..at + 8].try_into().unwrap());
    let i32_at = |at| i32::from_le_bytes(packed[at..at + 4].try_into().unwrap());
    Ok(Inputs {
        asked: i64_at(0),
        received: i64_at(8),
        air_drops: i64_at(16),
        from_state: i64_at(24),
        disruption: i64_at(68),
        division_demand: Demand {
            count: i32_at(32),
            value: i64_at(36),
        },
        airplane_demand: Demand {
            count: i32_at(44),
            value: i64_at(48),
        },
        ship_demand: Demand {
            count: i32_at(56),
            value: i64_at(60),
        },
        origins_len,
    })
}

fn capture_origins(
    memory: &impl Memory,
    pointer: usize,
    origins: &mut [Origin],
) -> Result<usize, Failure> {
    let address = pointer.checked_add(ORIGINS).ok_or(Failure::Unreadable)?;
    let header = read::<[u8; 16]>(memory, address).ok_or(Failure::Unreadable)?;
    let array = Array::from_header(header, ORIGIN_STRIDE).ok_or(Failure::InvalidOrigins)?;
    let count = array.count as usize;
    if count > origins.len() {
        return Err(Failure::OriginsCapacity { required: count });
    }
    const BATCH: usize = 64;
    let mut bytes = [0; BATCH * ORIGIN_BYTES];
    for (batch, destination) in origins[..count].chunks_mut(BATCH).enumerate() {
        let first = batch * BATCH;
        let source = array
            .element(first as u32, ORIGIN_STRIDE)
            .ok_or(Failure::InvalidOrigins)?;
        if memory.copy_records(
            RecordSources::Strided {
                base: source,
                stride: ORIGIN_STRIDE,
                count: destination.len(),
            },
            &ORIGIN_LAYOUT,
            &mut bytes[..destination.len() * ORIGIN_BYTES],
        ) != destination.len() * ORIGIN_FIELDS.len()
        {
            return Err(Failure::Unreadable);
        }
        for (origin, bytes) in destination.iter_mut().zip(bytes.chunks_exact(ORIGIN_BYTES)) {
            *origin = Origin {
                province: i32::from_le_bytes(bytes[16..20].try_into().unwrap()),
                requested: i64::from_le_bytes(bytes[..8].try_into().unwrap()),
                received: i64::from_le_bytes(bytes[8..16].try_into().unwrap()),
            };
        }
    }
    let current = read::<[u8; 16]>(memory, address).ok_or(Failure::Unreadable)?;
    if current != header {
        return Err(Failure::OriginsChanged);
    }
    Ok(count)
}

/// Off-hour sizing only: the same native origin header, without scalar or
/// payload acquisition. Hints never become recorded values.
pub fn origin_demand(memory: &impl Memory, pointer: usize) -> Result<usize, Failure> {
    let address = pointer
        .checked_add(ORIGINS)
        .filter(|_| pointer != 0)
        .ok_or(Failure::Unreadable)?;
    let header = read::<[u8; 16]>(memory, address).ok_or(Failure::Unreadable)?;
    let array = Array::from_header(header, ORIGIN_STRIDE).ok_or(Failure::InvalidOrigins)?;
    if read::<[u8; 16]>(memory, address) != Some(header) {
        return Err(Failure::OriginsChanged);
    }
    Ok(array.count as usize)
}

#[cfg(test)]
mod tests;
