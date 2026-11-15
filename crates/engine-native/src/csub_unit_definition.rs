//! Subunit-definition operands selected as the army stat block. Readers retain
//! exact native words; the caller owns the active-stat writer certificate and
//! the parked acquisition boundary.

use engine_sites::Rva;
/// Primary vtable.
pub const VTABLE_RVA: Rva = Rva(0x0280_7710);
/// CSubUnitDefinition cached defense, i64/100000.
pub const DEFENSE: usize = 0xa8;
/// CSubUnitDefinition cached breakthrough, i64/100000.
pub const BREAKTHROUGH: usize = 0xb0;
/// Effective definition combat width, before contextual country/commander factors.
pub const COMBAT_WIDTH: usize = 0x58;
/// CSubUnitDefinition cached hardness, i64/100000.
pub const HARDNESS: usize = 0xb8;
/// CSubUnitDefinition cached soft attack, i64/100000.
pub const SOFT_ATTACK: usize = 0xc0;
/// CSubUnitDefinition cached hard attack, i64/100000.
pub const HARD_ATTACK: usize = 0xc8;
/// CSubUnitDefinition cached max organisation, i64/100000.
pub const MAXIMUM_ORGANISATION: usize = 0x280;
/// CSubUnitDefinition cached max strength, i64/100000.
pub const MAXIMUM_STRENGTH: usize = 0x288;
/// Sub-unit definition maximum speed, signed i64 fixed5; division movement-speed calculation starts
/// from `MAXIMUM_SPEED` in its stat block.
pub const MAXIMUM_SPEED: usize = 0x290;
/// Raw fixed5 weight consumed from the original stat block's `TRANSPORT_WEIGHT` by the unit weight
/// getter. Transport demand rounds each selected occurrence independently before signed32
/// narrowing. No derived template/population count substitutes for this cache.
pub const TRANSPORT_WEIGHT: usize = 0x2b8;
/// Coastal-exfiltration byte initialized true and ANDed across grouped
/// subunits. Active-stat updates retain it in the army block. It permits
/// embarkation from a coastal province into port sea without a naval base.
pub const CAN_EXFILTRATE_FROM_COAST: usize = 0x5c0;

/// Effective army maxima include their minimum of 100.
pub fn division_maxima(
    memory: &impl crate::Memory,
    base: usize,
    pointer: usize,
) -> Result<(i64, i64), &'static str> {
    Ok(capture_division_maxima(memory, base, pointer)?.effective())
}

/// Owned exact cached operands. Clamping belongs to projection.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DivisionMaxima {
    pub pointer: usize,
    pub organisation: i64,
    pub strength: i64,
    /// The native [`CAN_EXFILTRATE_FROM_COAST`] byte, not normalized.
    pub can_exfiltrate_from_coast: u8,
}

/// The vtable, then both cached maxima and the exfiltration byte.
const MAXIMA_FIELDS: [crate::memory::RecordField; 3] = [
    crate::memory::RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    crate::memory::RecordField {
        source_offset: MAXIMUM_ORGANISATION,
        destination_offset: 8,
        len: 16,
    },
    crate::memory::RecordField {
        source_offset: CAN_EXFILTRATE_FROM_COAST,
        destination_offset: 24,
        len: 1,
    },
];
const MAXIMA_LAYOUT: crate::memory::RecordLayout<'static> =
    crate::memory::RecordLayout::new(&MAXIMA_FIELDS, 25).expect("valid maxima layout");

/// Up to 64 distinct, unheld blocks selected from authenticated census seeds. Organisation and
/// strength maxima follow the division's stat block; exfiltration eligibility consumes its byte.
/// The status walk still authenticates each seed before taking its block and inserts it into the
/// original memo in order.
pub(crate) struct MaximaBatch {
    bytes: [u8; 64 * 25],
    complete: [bool; 64],
    len: usize,
}

impl MaximaBatch {
    pub(crate) fn new() -> Self {
        Self {
            bytes: [0; 64 * 25],
            complete: [false; 64],
            len: 0,
        }
    }

    pub(crate) fn capture(&mut self, memory: &impl crate::Memory, pointers: &[usize]) {
        self.len = pointers.len();
        crate::memory::gather_records_once(
            memory,
            pointers,
            &MAXIMA_LAYOUT,
            &mut self.bytes[..self.len * 25],
            &mut self.complete,
            crate::batch_fallback::Batch::DivisionMaxima,
        );
    }

    pub(crate) fn read(
        &self,
        memory: &impl crate::Memory,
        base: usize,
        pointer: usize,
        row: usize,
    ) -> Result<DivisionMaxima, &'static str> {
        if row < self.len && self.complete[row] {
            return decode_division_maxima(
                base,
                pointer,
                &self.bytes[row * 25..][..25],
                MAXIMA_FIELDS.len(),
            );
        }
        capture_division_maxima(memory, base, pointer)
    }
}

/// The maxima of each pointer, as the status pass's ordered admission reads them: gathered in
/// batches, each incomplete record read alone.
pub fn stage_division_maxima(
    memory: &impl crate::Memory,
    base: usize,
    pointers: &[usize],
    out: &mut [Result<DivisionMaxima, &'static str>],
) {
    let mut batch = MaximaBatch::new();
    for (pointers, out) in pointers.chunks(64).zip(out.chunks_mut(64)) {
        batch.capture(memory, pointers);
        for (row, (&pointer, out)) in pointers.iter().zip(out).enumerate() {
            *out = batch.read(memory, base, pointer, row);
        }
    }
}

impl DivisionMaxima {
    pub fn effective(self) -> (i64, i64) {
        (self.strength.max(100), self.organisation.max(100))
    }
}

pub fn capture_division_maxima(
    memory: &impl crate::Memory,
    base: usize,
    pointer: usize,
) -> Result<DivisionMaxima, &'static str> {
    engine_sites::at(base, VTABLE_RVA)
        .ok_or("division stat block identity mismatch")?;
    if pointer == 0 {
        return Err("division stat block identity mismatch");
    }
    // One guarded copy of the vtable and both cached spans; the spans are
    // used only once the vtable authenticates the block.
    let mut bytes = [0_u8; 25];
    let copied = memory.copy_records(
        crate::memory::RecordSources::Pointers(&[pointer]),
        &MAXIMA_LAYOUT,
        &mut bytes,
    );
    decode_division_maxima(base, pointer, &bytes, copied)
}

fn decode_division_maxima(
    base: usize,
    pointer: usize,
    bytes: &[u8],
    copied: usize,
) -> Result<DivisionMaxima, &'static str> {
    let vtable = engine_sites::at(base, VTABLE_RVA)
        .ok_or("division stat block identity mismatch")? as u64;
    if copied == 0 || u64::from_le_bytes(bytes[..8].try_into().unwrap()) != vtable {
        return Err("division stat block identity mismatch");
    }
    if copied < MAXIMA_FIELDS.len() {
        return Err("division maxima unreadable");
    }
    Ok(DivisionMaxima {
        pointer,
        organisation: i64::from_le_bytes(bytes[8..16].try_into().unwrap()),
        strength: i64::from_le_bytes(bytes[16..24].try_into().unwrap()),
        can_exfiltrate_from_coast: bytes[24],
    })
}

/// Exact cached defense and soft/hard attack for demanded class-zero armies;
/// these values are not reconstructed.
pub fn capture_division_combat_stats(
    memory: &impl crate::Memory,
    base: usize,
    pointer: usize,
) -> Result<[i64; 3], &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA)
        .ok_or("division combat stat module overflow")? as u64;
    if pointer == 0 || crate::read::<u64>(memory, pointer) != Some(expected) {
        return Err("division combat stat block identity mismatch");
    }
    let mut defense = [0; 8];
    let mut attacks = [0; 16];
    let defense_at = pointer
        .checked_add(0xa8)
        .ok_or("division combat stat address overflow")?;
    let attacks_at = pointer
        .checked_add(0xc0)
        .ok_or("division combat stat address overflow")?;
    if memory.copy_scattered_prefetched([defense_at, attacks_at], &mut [&mut defense, &mut attacks])
        != 2
    {
        return Err("division border combat stats unreadable");
    }
    Ok([
        i64::from_le_bytes(defense),
        i64::from_le_bytes(attacks[..8].try_into().unwrap()),
        i64::from_le_bytes(attacks[8..].try_into().unwrap()),
    ])
}
/// CSubUnitDefinition base fuel i64.
pub const BASE_FUEL: usize = 0x2c8;
/// CSubUnitDefinition token u32.
pub const TOKEN: usize = 0x08;
/// Real/null discriminator byte: one for real definitions, zero for null subtype.
pub const IS_REAL: usize = 0x10;
/// CSubUnitDefinition token-name MSVC string.
pub const NAME: usize = 0x18;
/// CSubUnitDefinition embedded archetype requirements.
pub const ARCHETYPE_REQUIREMENTS: usize = 0x310;
/// Equipment-requirements pointer vector; equipment selection follows its
/// first element rather than treating the vector pointer as equipment.
pub const ARCHETYPE_REQUIREMENTS_DATA: usize = 0x318;
/// Each requirement stores an archetype pointer and signed i64 fixed5 amount.
pub const ARCHETYPE_REQUIREMENT_STRIDE: usize = 0x10;
/// Global database entry index retained by country-local clones. The object
/// is global only when the database holds this exact pointer at that index.
pub const DATABASE_INDEX: usize = 0x58c;
/// Global subunit database with indexed definition pointers and null entry zero.
pub const DATABASE_RVA: Rva = Rva(0x033d_d4d8);
/// The database's CPdxArray of definition pointers.
pub const DATABASE_VECTOR: usize = 0x40;

/// The global subunit database, read once per walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Database(crate::cpdx_array::Array);

impl Database {
    pub fn read(memory: &impl crate::Memory, base: usize) -> Result<Self, &'static str> {
        let database = engine_sites::at(base, DATABASE_RVA)
            .and_then(|at| crate::read::<u64>(memory, at))
            .and_then(|at| usize::try_from(at).ok())
            .filter(|at| *at != 0)
            .ok_or("subunit database pointer unreadable or null")?;
        database
            .checked_add(DATABASE_VECTOR)
            .and_then(|at| crate::cpdx_array::Array::read(memory, at, 8))
            .filter(|vector| vector.count > 0)
            .map(Self)
            .ok_or("subunit database vector invalid")
    }

    /// The database's pointer vector, indexed by `DATABASE_INDEX`.
    pub fn vector(self) -> crate::cpdx_array::Array {
        self.0
    }

    /// The global definition at a [`DATABASE_INDEX`] value already read.
    pub fn entry(self, memory: &impl crate::Memory, index: i32) -> Result<usize, &'static str> {
        u32::try_from(index)
            .ok()
            .filter(|index| *index != 0)
            .and_then(|index| self.0.element(index, 8))
            .and_then(|at| crate::read::<u64>(memory, at))
            .and_then(|global| usize::try_from(global).ok())
            .filter(|global| *global != 0)
            .ok_or("subunit database index outside the database")
    }
}

/// One ordered archetype requirement of a definition (`ARCHETYPE_REQUIREMENTS` pool).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Requirement {
    /// The archetype CEquipmentType; compared by identity, never published.
    pub archetype: usize,
    pub amount: i64,
}

/// The requirement vector's header of a real definition.
pub fn requirements(
    memory: &impl crate::Memory,
    definition: usize,
) -> Result<crate::cpdx_array::Array, &'static str> {
    definition
        .checked_add(ARCHETYPE_REQUIREMENTS_DATA)
        .and_then(|at| crate::cpdx_array::Array::read(memory, at, ARCHETYPE_REQUIREMENT_STRIDE))
        .ok_or("subunit requirement vector invalid")
}

/// Entry `index` of a requirement vector.
pub fn requirement(
    memory: &impl crate::Memory,
    vector: crate::cpdx_array::Array,
    index: u32,
) -> Result<Requirement, &'static str> {
    let bytes = vector
        .element(index, ARCHETYPE_REQUIREMENT_STRIDE)
        .and_then(|at| crate::read::<[u8; 16]>(memory, at))
        .ok_or("subunit requirement unreadable")?;
    Ok(Requirement {
        archetype: usize::try_from(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
            .ok()
            .filter(|at| *at != 0)
            .ok_or("subunit requirement archetype null")?,
        amount: i64::from_le_bytes(bytes[8..].try_into().unwrap()),
    })
}
/// Equipment-category mask, u64, accumulated from authored type masks and
/// tested through the army stat block. Air-equipment selection reads its air bits.
pub const AIR_FLAGS: usize = 0x5a8;
/// Category dword and flags consumed by category selection, separate from sprite frame.
pub const CATEGORY: usize = 0x5b0;
/// The sub-unit's own modifier key, u32 (zero is none): the land movement speed
/// adds the owner country's accumulator entry for it when the sub-unit is a
/// division's movement priority sub-unit.
pub const SPEED_MODIFIER_KEY: usize = 0x5d8;
/// Vector of the sub-unit's category pointers, 8-byte elements in content order.
pub const CATEGORIES: usize = 0x508;
/// A sub-unit category's modifier key, u32 (zero is none): the land movement
/// speed adds the owner country's accumulator entry for it, once per listing.
pub const CATEGORY_MODIFIER_KEY: usize = 0x60;

pub(crate) fn template_category(
    memory: &impl crate::Memory,
    pointer: usize,
) -> Result<i32, &'static str> {
    let bytes = pointer
        .checked_add(CATEGORY)
        .and_then(|p| crate::read::<[u8; CATEGORY_LEN]>(memory, p))
        .ok_or("subunit category unreadable")?;
    Ok(category_of(&bytes))
}

/// The bytes [`template_category`] reads from `CATEGORY`.
pub(crate) const CATEGORY_LEN: usize = 13;

/// [`template_category`] of the `CATEGORY_LEN` bytes at `CATEGORY`.
pub(crate) fn category_of(bytes: &[u8; CATEGORY_LEN]) -> i32 {
    if bytes[8] != 0 {
        9
    } else if bytes[12] != 0 {
        4
    } else if bytes[10] != 0 {
        5
    } else if bytes[5] != 0 {
        6
    } else {
        match i32::from_le_bytes(bytes[..4].try_into().unwrap()) {
            1 => 3,
            2 => 8,
            3 => 7,
            4 => 1,
            9 => 2,
            10 => 10,
            20 => 6,
            29 => 4,
            30 => 5,
            31 => 9,
            _ => 0,
        }
    }
}
/// Subunit-definition constructor, also used for the null subtype.
pub const CONSTRUCTOR_RVA: Rva = Rva(0x0109_ee10);
/// Null-subtype singleton pointer.
pub const NULL_OBJECT_RVA: Rva = Rva(0x033e_7be0);
/// Null-singleton getter used by default air-wing pools.
pub const GET_NULL_OBJECT_RVA: Rva = Rva(0x00bf_1d50);
/// Null-subtype primary vtable.
pub const NULL_VTABLE_RVA: Rva = Rva(0x029c_1178);
/// Persistent null-subunit lexer token, checked against its loaded spelling.
pub const NULL_TOKEN: u32 = 0x165;

/// Embedded stat-block terrain-adjuster array.
pub const MOVEMENT_TERRAIN_ADJUSTERS: usize = 0x3f8;
/// Cached anti-air stat used by combat power calculation.
pub const MOVEMENT_AIR_DEFENSE: usize = 0x220;
/// Combat anti-air denominator define, signed fixed point.
pub const AIR_DEFENSE_DENOMINATOR_RVA: Rva = Rva(0x033e_18b8);
/// Combat anti-air maximum-reduction define, signed fixed point.
pub const AIR_DEFENSE_REDUCTION_RVA: Rva = Rva(0x033e_1830);

fn movement_identity(
    memory: &impl crate::Memory,
    base: usize,
    stats: usize,
) -> Result<(), &'static str> {
    if stats == 0
        || crate::read::<u64>(memory, stats) != engine_sites::at(base, VTABLE_RVA).map(|p| p as u64)
    {
        Err("movement stat block identity mismatch")
    } else {
        Ok(())
    }
}

/// Passive movement component including the terrain static adjuster. Stat
/// block and terrain fields share one guarded call; the adjuster entry uses
/// another. Reach fields only at their original gate so a failure beyond an
/// earlier mismatch or failure is never reported.
pub fn movement_terrain(
    memory: &impl crate::Memory,
    base: usize,
    stats: usize,
    terrain: usize,
) -> Result<i64, &'static str> {
    // Authenticate terrain type and its embedded unit adjuster.
    const TERRAIN_VTABLE: Rva = Rva(0x02a3_b470);
    const ADJUSTER_VTABLE: Rva = Rva(0x0280_75d8);
    const UNREADABLE: &str = "movement stat operand unreadable";
    // A null or overflowing leaf is an unreadable operand without a read; the
    // scattered copy stops at its zero address exactly where the leaf stops.
    let leaf = |pointer: usize, offset: usize| {
        pointer
            .checked_add(offset)
            .filter(|_| pointer != 0)
            .unwrap_or(0)
    };
    let array_at = stats.checked_add(MOVEMENT_TERRAIN_ADJUSTERS);
    let (mut identity, mut vtable, mut index) = ([0_u8; 8], [0_u8; 8], [0_u8; 4]);
    let (mut header, mut adjuster, mut token, mut speed) =
        ([0_u8; 16], [0_u8; 8], [0_u8; 4], [0_u8; 8]);
    let addresses = [
        stats,
        leaf(terrain, 0),
        leaf(terrain, 0x88),
        array_at.unwrap_or(0),
        leaf(terrain, 0x168),
        leaf(terrain, 8),
        leaf(terrain, 0x188),
    ];
    let read = memory.copy_scattered_prefetched(
        addresses,
        &mut [
            &mut identity,
            &mut vtable,
            &mut index,
            &mut header,
            &mut adjuster,
            &mut token,
            &mut speed,
        ],
    );
    if stats == 0
        || (read > 0).then(|| u64::from_le_bytes(identity))
            != engine_sites::at(base, VTABLE_RVA).map(|p| p as u64)
    {
        return Err("movement stat block identity mismatch");
    }
    if read < 2 {
        return Err(UNREADABLE);
    }
    if u64::from_le_bytes(vtable)
        != engine_sites::at(base, TERRAIN_VTABLE)
            .ok_or("terrain vtable overflow")? as u64
    {
        return Err("movement terrain identity mismatch");
    }
    if read < 3 {
        return Err(UNREADABLE);
    }
    // Terrain indices are one-based, excluding the database null slot.
    let index = i32::from_le_bytes(index);
    array_at.ok_or("terrain array overflow")?;
    let array = (read > 3)
        .then(|| crate::cpdx_array::Array::from_header(header, 0x28))
        .flatten()
        .ok_or("movement terrain array unreadable")?;
    let entry = u32::try_from(index)
        .ok()
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| array.element(index, 0x28))
        .ok_or("movement terrain index outside stat cache")?;
    let expected = engine_sites::at(base, ADJUSTER_VTABLE)
        .ok_or("adjuster vtable overflow")? as u64;
    let (mut entry_vtable, mut entry_token, mut entry_speed) = ([0_u8; 8], [0_u8; 4], [0_u8; 8]);
    let entry_addresses = [leaf(entry, 0), leaf(entry, 8), leaf(entry, 0x20)];
    let entry_read = memory.copy_scattered_prefetched(
        entry_addresses,
        &mut [&mut entry_vtable, &mut entry_token, &mut entry_speed],
    );
    let mismatch = "movement terrain adjuster identity mismatch";
    if entry_read < 1 {
        return Err(UNREADABLE);
    }
    if u64::from_le_bytes(entry_vtable) != expected {
        return Err(mismatch);
    }
    if read < 5 {
        return Err(UNREADABLE);
    }
    if u64::from_le_bytes(adjuster) != expected {
        return Err(mismatch);
    }
    if entry_read < 2 || read < 6 {
        return Err(UNREADABLE);
    }
    if entry_token != token {
        return Err(mismatch);
    }
    if entry_read < 3 || read < 7 {
        return Err(UNREADABLE);
    }
    Ok(i64::from_le_bytes(entry_speed).wrapping_add(i64::from_le_bytes(speed)))
}

/// Pure native integer calculation. These three signed operands are its entire
/// runtime dependency set; memoization must key all of them without normalizing
/// signs, wrapping values, or exceptional inputs.
pub fn movement_air_defense_value(
    defense: i64,
    denominator: i64,
    reduction: i64,
) -> Result<i64, &'static str> {
    if defense == 0 {
        return Ok(0);
    }
    // The exponent low bit sends negative inputs through absolute value.
    // The minimum signed value wraps negative, and the power result is zero.
    let power = crate::fixed5::power(defense, 150_000)?;
    let ratio = crate::fixed5::fraction(power, power.wrapping_add(denominator))?;
    Ok(ratio.wrapping_mul(reduction) / crate::fixed5::SCALE)
}

#[cfg(test)]
mod movement_tests {
    use super::*;

    #[test]
    fn integer_series_exact_identity_and_zero() {
        assert_eq!(crate::fixed5::log(100_000), Ok(0));
        assert_eq!(crate::fixed5::exp(0), Ok(100_000));
        assert_eq!(movement_air_defense_value(0, 100_000, 50_000), Ok(0));
        assert_eq!(
            movement_air_defense_value(100_000, 100_000, 50_000),
            Ok(25_000)
        );
        assert_eq!(
            movement_air_defense_value(-100_000, 100_000, 50_000),
            Ok(25_000)
        );
    }

    #[test]
    fn exceptional_fraction_and_wrapping_product_are_native() {
        assert_eq!(crate::fixed5::fraction(1, 0), Ok(0xffff_ffff));
        assert_eq!(crate::fixed5::fraction(i64::MIN, 100_000), Ok(0xffff_ffff));
        assert_eq!(crate::fixed5::fraction(i64::MAX, 100_000), Ok(0xffff_ffff));
        let value = i64::MAX / 2;
        assert_eq!(
            crate::fixed5::fraction(value, 100_000),
            Ok(value.wrapping_mul(100_000) / 100_000)
        );
    }

    #[test]
    fn fractional_power_preserves_native_series_rounding() {
        assert_eq!(crate::fixed5::log(50_000), Ok(-69_308));
        assert_eq!(crate::fixed5::log(1_000_000), Ok(230_258));
        assert_eq!(
            movement_air_defense_value(50_000, 100_000, 50_000),
            Ok(13_061)
        );
        assert_eq!(
            movement_air_defense_value(150_000, 100_000, 50_000),
            Ok(32_376)
        );
        assert_eq!(
            movement_air_defense_value(1_000_000, 100_000, 50_000),
            Ok(48_467)
        );
    }

    #[test]
    fn terrain_one_based_cache_static_sum_and_invalid_identity() {
        use crate::memory::SliceMemory;
        let mut bytes = [0_u8; 0x1000];
        let base = 0x140_000_000_usize;
        let stats = 0x100;
        let terrain = 0x600;
        let cache = 0x900;
        bytes[stats..stats + 8].copy_from_slice(&((base + VTABLE_RVA.0) as u64).to_le_bytes());
        bytes[stats + MOVEMENT_TERRAIN_ADJUSTERS..stats + MOVEMENT_TERRAIN_ADJUSTERS + 8]
            .copy_from_slice(&(cache as u64).to_le_bytes());
        bytes[stats + MOVEMENT_TERRAIN_ADJUSTERS + 8..stats + MOVEMENT_TERRAIN_ADJUSTERS + 12]
            .copy_from_slice(&1_u32.to_le_bytes());
        bytes[stats + MOVEMENT_TERRAIN_ADJUSTERS + 12..stats + MOVEMENT_TERRAIN_ADJUSTERS + 16]
            .copy_from_slice(&1_u32.to_le_bytes());
        bytes[terrain..terrain + 8].copy_from_slice(&((base + 0x02a3_b470) as u64).to_le_bytes());
        bytes[terrain + 8..terrain + 12].copy_from_slice(&123_u32.to_le_bytes());
        bytes[terrain + 0x88..terrain + 0x8c].copy_from_slice(&1_i32.to_le_bytes());
        for pointer in [cache, terrain + 0x168] {
            bytes[pointer..pointer + 8]
                .copy_from_slice(&((base + 0x0280_75d8) as u64).to_le_bytes());
        }
        bytes[cache + 8..cache + 12].copy_from_slice(&123_u32.to_le_bytes());
        bytes[cache + 0x20..cache + 0x28].copy_from_slice(&i64::MAX.to_le_bytes());
        bytes[terrain + 0x188..terrain + 0x190].copy_from_slice(&1_i64.to_le_bytes());
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(
            movement_terrain(&memory, base, stats, terrain),
            Ok(i64::MIN)
        );
        bytes[terrain + 0x88..terrain + 0x8c].copy_from_slice(&0_i32.to_le_bytes());
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert!(movement_terrain(&memory, base, stats, terrain).is_err());
        bytes[terrain + 0x88..terrain + 0x8c].copy_from_slice(&1_i32.to_le_bytes());
        bytes[cache + 8..cache + 12].copy_from_slice(&124_u32.to_le_bytes());
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert!(movement_terrain(&memory, base, stats, terrain).is_err());
        assert!(movement_terrain(&memory, base, 0, terrain).is_err());
    }
}

/// The `CUnitAdjuster` vtable of each entry of [`MOVEMENT_TERRAIN_ADJUSTERS`]
/// and of a terrain's own adjuster.
pub const UNIT_ADJUSTER_VTABLE_RVA: Rva = Rva(0x0280_75d8);
/// Unit adjuster stride and its attack component, used by additive adjustment.
pub const UNIT_ADJUSTER_STRIDE: usize = 0x28;
pub const UNIT_ADJUSTER_ATTACK: usize = 0x10;
/// The attack list's stat-block scalars: night (`ATTACK_NIGHT`), fort (`ATTACK_FORT` pair), river
/// (`ATTACK_RIVER` pair) and snow (`ATTACK_SNOW`). One span from the night term through the terrain
/// array header reads them all.
pub const ATTACK_NIGHT: usize = 0x340;
pub const ATTACK_FORT: usize = 0x368;
pub const ATTACK_RIVER: usize = 0x390;
pub const ATTACK_SNOW: usize = 0x3e0;
const ATTACK_SPAN: usize = MOVEMENT_TERRAIN_ADJUSTERS + 16 - ATTACK_NIGHT;
/// A unit adjuster's movement component.
pub const UNIT_ADJUSTER_MOVEMENT: usize = 0x20;
/// The embedded fort, river and snow unit adjusters the attack span covers; each is authenticated
/// by its own adjuster vtable before its movement word is kept. The movement speed adds the fort
/// word on an edge into a fortified province, the river word as a crossing's adjustment and the
/// snow word on an edge into snow.
pub const MOVEMENT_ADJUSTERS: [usize; 3] = [0x358, 0x380, 0x3d0];
/// The stat-block word the movement speed's fuel ratio takes its capacity from,
/// scaled by a define.
pub const MOVEMENT_FUEL_RATE: usize = 0x120;

/// Native stat selectors and their deployed active-cache scalar words.
/// This is the panel's cache, independent of template designer reconstruction.
pub const PANEL_CACHE_FIELDS: [(u8, usize); 23] = [
    (0x3d, MAXIMUM_STRENGTH),
    (0x3c, MAXIMUM_ORGANISATION),
    (0, 0xa0),
    (6, 0xd0),
    (0xb, 0xf8),
    (0x41, 0x2a8),
    (0x42, 0x2b0),
    (9, 0xe8),
    (0xe, 0x110),
    (4, SOFT_ATTACK),
    (5, HARD_ATTACK),
    (0x30, MOVEMENT_AIR_DEFENSE),
    (1, DEFENSE),
    (2, BREAKTHROUGH),
    (0x3f, 0x298),
    (0x40, 0x2a0),
    (8, 0xe0),
    (7, 0xd8),
    (0xf, 0x118),
    (0x43, TRANSPORT_WEIGHT),
    (0x45, 0x2c8),
    (3, HARDNESS),
    (0x3e, MAXIMUM_SPEED),
];

/// The stat block's suppression word and its stat selector: kept beside the panel's cache as the
/// division designer's witness of its suppression row. The panel does not show it.
pub const SUPPRESSION_CACHE_FIELD: (u8, usize) = (0xc, 0x100);

/// A stat block's attack operands: the attack of each terrain adjuster entry in
/// array order (entry `i` serves terrain database index `i + 1`) and the
/// scalars, each an exact fixed5 word.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AttackStats {
    /// Deployed panel scalar cache, in PANEL_CACHE_FIELDS order.
    pub panel: [Option<i64>; 23],
    /// [`SUPPRESSION_CACHE_FIELD`]'s raw word, read in the same copy as the panel's words.
    pub suppression: Option<i64>,
    pub terrain: Vec<i64>,
    pub river: [i64; 2],
    pub fort: [i64; 2],
    pub snow: i64,
    pub night: i64,
    /// Each terrain adjuster entry's movement, parallel to `terrain`.
    pub movement_terrain: Vec<i64>,
    /// The fort, river and snow adjusters' movement words; `None` when an
    /// embedded adjuster did not authenticate.
    pub movement: Option<[i64; 3]>,
    /// [`MOVEMENT_FUEL_RATE`] and [`MOVEMENT_AIR_DEFENSE`], raw.
    pub fuel_rate: i64,
    pub air_defense: i64,
}

/// Reads [`AttackStats`] of an authenticated stat block: one span for the
/// scalars and the terrain array header, then the entries 32 at a time, each
/// authenticated as a `CUnitAdjuster`. The entries reuse `terrain`'s storage.
pub fn capture_attack_stats(
    memory: &impl crate::Memory,
    base: usize,
    stats: usize,
    terrain: &mut Vec<i64>,
    movement_terrain: &mut Vec<i64>,
) -> Result<AttackStats, &'static str> {
    capture_attack_stats_holding(memory, base, stats, terrain, movement_terrain, None)
}

/// The same read reusing the status pass's raw maxima of this exact stat block.
pub fn capture_attack_stats_holding(
    memory: &impl crate::Memory,
    base: usize,
    stats: usize,
    terrain: &mut Vec<i64>,
    movement_terrain: &mut Vec<i64>,
    maxima: Option<DivisionMaxima>,
) -> Result<AttackStats, &'static str> {
    movement_identity(memory, base, stats)?;
    let mut span = [0_u8; ATTACK_SPAN];
    let at = stats
        .checked_add(ATTACK_NIGHT)
        .ok_or("stat block address overflow")?;
    if !crate::read_slice(memory, at, &mut span) {
        return Err("stat block attack operands unreadable");
    }
    let word = |offset: usize| {
        let at = offset - ATTACK_NIGHT;
        i64::from_le_bytes(span[at..at + 8].try_into().unwrap())
    };
    // The fuel and anti-air words lie outside the span: one more guarded copy.
    let (mut fuel_rate, mut air_defense) = ([0_u8; 8], [0_u8; 8]);
    let words = [
        stats
            .checked_add(MOVEMENT_FUEL_RATE)
            .ok_or("stat block address overflow")?,
        stats
            .checked_add(MOVEMENT_AIR_DEFENSE)
            .ok_or("stat block address overflow")?,
    ];
    if memory.copy_scattered(words, &mut [&mut fuel_rate, &mut air_defense]) != 2 {
        return Err("stat block movement operands unreadable");
    }
    // Reuse the anti-air word already acquired above; acquire only missing
    // scalar words of this new/dirty cache, then the suppression word, in one
    // guarded scattered copy.
    let mut panel_bytes = [[0_u8; 8]; 23];
    let mut panel_addresses = [0_usize; 23];
    let mut next = 0;
    for (index, (_, offset)) in PANEL_CACHE_FIELDS.iter().enumerate() {
        if index != 11 {
            panel_addresses[next] = stats
                .checked_add(*offset)
                .ok_or("stat block address overflow")?;
            next += 1;
        }
    }
    panel_addresses[next] = stats
        .checked_add(SUPPRESSION_CACHE_FIELD.1)
        .ok_or("stat block address overflow")?;
    let maxima = maxima.filter(|maxima| maxima.pointer == stats);
    let mut destinations = panel_bytes.each_mut().map(|bytes| &mut bytes[..]);
    if maxima.is_some() {
        destinations[0] = &mut [];
        destinations[1] = &mut [];
    }
    let complete = memory.copy_scattered(panel_addresses, &mut destinations);
    let mut panel = [None; 23];
    panel[11] = Some(i64::from_le_bytes(air_defense));
    let mut next = 0;
    for (index, value) in panel.iter_mut().enumerate() {
        if index != 11 {
            if next < complete {
                *value = Some(i64::from_le_bytes(panel_bytes[next]));
            }
            next += 1;
        }
    }
    let suppression = (complete == panel_bytes.len()).then(|| i64::from_le_bytes(panel_bytes[22]));
    if let Some(maxima) = maxima {
        panel[0] = Some(maxima.strength);
        panel[1] = Some(maxima.organisation);
    }
    let header = MOVEMENT_TERRAIN_ADJUSTERS - ATTACK_NIGHT;
    let array = crate::cpdx_array::Array::from_header(
        span[header..header + 16].try_into().unwrap(),
        UNIT_ADJUSTER_STRIDE,
    )
    .ok_or("stat block terrain adjuster array invalid")?;
    let expected = engine_sites::at(base, UNIT_ADJUSTER_VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    terrain.clear();
    movement_terrain.clear();
    crate::allocation::reserve_exact(terrain, array.count as usize)
        .map_err(|_| "stat block terrain allocation failed")?;
    crate::allocation::reserve_exact(movement_terrain, array.count as usize)
        .map_err(|_| "stat block terrain allocation failed")?;
    const BATCH: usize = 32;
    let mut entries = [0_u8; BATCH * UNIT_ADJUSTER_STRIDE];
    let mut first = 0;
    while first < array.count {
        let len = (array.count - first).min(BATCH as u32) as usize;
        let address = array
            .element(first, UNIT_ADJUSTER_STRIDE)
            .ok_or("stat block terrain adjuster address overflow")?;
        if !crate::read_slice(memory, address, &mut entries[..len * UNIT_ADJUSTER_STRIDE]) {
            return Err("stat block terrain adjusters unreadable");
        }
        for entry in entries[..len * UNIT_ADJUSTER_STRIDE].chunks_exact(UNIT_ADJUSTER_STRIDE) {
            if u64::from_le_bytes(entry[..8].try_into().unwrap()) != expected {
                return Err("stat block terrain adjuster identity mismatch");
            }
            terrain.push(i64::from_le_bytes(
                entry[UNIT_ADJUSTER_ATTACK..UNIT_ADJUSTER_ATTACK + 8]
                    .try_into()
                    .unwrap(),
            ));
            movement_terrain.push(i64::from_le_bytes(
                entry[UNIT_ADJUSTER_MOVEMENT..UNIT_ADJUSTER_MOVEMENT + 8]
                    .try_into()
                    .unwrap(),
            ));
        }
        first += len as u32;
    }
    let movement = MOVEMENT_ADJUSTERS
        .iter()
        .all(|&adjuster| word(adjuster) as u64 == expected)
        .then(|| MOVEMENT_ADJUSTERS.map(|adjuster| word(adjuster + UNIT_ADJUSTER_MOVEMENT)));
    Ok(AttackStats {
        panel,
        suppression,
        terrain: std::mem::take(terrain),
        river: [word(ATTACK_RIVER), word(ATTACK_RIVER + 8)],
        fort: [word(ATTACK_FORT), word(ATTACK_FORT + 8)],
        snow: word(ATTACK_SNOW),
        night: word(ATTACK_NIGHT),
        movement_terrain: std::mem::take(movement_terrain),
        movement,
        fuel_rate: i64::from_le_bytes(fuel_rate),
        air_defense: i64::from_le_bytes(air_defense),
    })
}

#[cfg(test)]
mod attack_tests {
    use super::*;
    use crate::memory::SliceMemory;

    const BASE: usize = 0x1000;
    const STATS: usize = 0x2000;
    const ENTRIES: usize = 0x3000;

    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at - BASE..at - BASE + value.len()].copy_from_slice(value);
    }

    fn memory(entries: u32) -> Vec<u8> {
        let mut bytes = vec![0_u8; 0x4000];
        put(
            &mut bytes,
            STATS,
            &((BASE + VTABLE_RVA.0) as u64).to_le_bytes(),
        );
        for (offset, value) in [
            (ATTACK_NIGHT, -50_000_i64),
            (ATTACK_FORT, 1),
            (ATTACK_FORT + 8, 2),
            (ATTACK_RIVER, -30_000),
            (ATTACK_RIVER + 8, i64::MIN),
            (ATTACK_SNOW, i64::MAX),
        ] {
            put(&mut bytes, STATS + offset, &value.to_le_bytes());
        }
        let header = STATS + MOVEMENT_TERRAIN_ADJUSTERS;
        put(&mut bytes, header, &(ENTRIES as u64).to_le_bytes());
        put(&mut bytes, header + 8, &entries.to_le_bytes());
        put(&mut bytes, header + 12, &entries.to_le_bytes());
        for index in 0..entries as usize {
            let entry = ENTRIES + index * UNIT_ADJUSTER_STRIDE;
            put(
                &mut bytes,
                entry,
                &((BASE + UNIT_ADJUSTER_VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                entry + UNIT_ADJUSTER_ATTACK,
                &(-(index as i64) * 1_000).to_le_bytes(),
            );
        }
        bytes
    }

    #[test]
    fn deployed_cache_reuses_raw_maxima_without_reading_their_words_again() {
        struct Guard<'a>(SliceMemory<'a>);
        impl crate::Memory for Guard<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                assert!(
                    !(STATS + MAXIMUM_ORGANISATION..STATS + MAXIMUM_STRENGTH + 8)
                        .contains(&address),
                    "raw maxima reread"
                );
                crate::Memory::copy(&self.0, address, output)
            }
        }
        let bytes = memory(0);
        let view = Guard(SliceMemory {
            base: BASE,
            bytes: &bytes,
        });
        let held = DivisionMaxima {
            pointer: STATS,
            strength: 1,
            organisation: -2,
            ..Default::default()
        };
        let stats = capture_attack_stats_holding(
            &view,
            BASE,
            STATS,
            &mut Vec::new(),
            &mut Vec::new(),
            Some(held),
        )
        .unwrap();
        assert_eq!(&stats.panel[..2], &[Some(1), Some(-2)]);
    }

    #[test]
    fn deployed_cache_keeps_every_raw_panel_word() {
        let mut bytes = memory(0);
        for (index, &(_, offset)) in PANEL_CACHE_FIELDS
            .iter()
            .chain([&SUPPRESSION_CACHE_FIELD])
            .enumerate()
        {
            put(
                &mut bytes,
                STATS + offset,
                &(i64::MIN + index as i64).to_le_bytes(),
            );
        }
        let view = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let stats =
            capture_attack_stats(&view, BASE, STATS, &mut Vec::new(), &mut Vec::new()).unwrap();
        for (index, value) in stats.panel.iter().enumerate() {
            assert_eq!(
                *value,
                Some(i64::MIN + index as i64),
                "panel scalar {index}"
            );
        }
        assert_eq!(stats.suppression, Some(i64::MIN + 23));
    }

    /// Each cached word lies at the stat block's stat array, eight bytes per selector: the
    /// suppression word beside the panel's own.
    #[test]
    fn cached_words_follow_their_selectors() {
        for &(selector, offset) in PANEL_CACHE_FIELDS.iter().chain([&SUPPRESSION_CACHE_FIELD]) {
            assert_eq!(offset, 0xa0 + 8 * usize::from(selector), "selector {selector}");
        }
    }

    #[test]
    fn attack_operands_read_every_entry_and_exact_scalars() {
        // Not a multiple of the batch: the last batch is short.
        let bytes = memory(35);
        let memory = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let mut scratch = Vec::new();
        let stats =
            capture_attack_stats(&memory, BASE, STATS, &mut scratch, &mut Vec::new()).unwrap();
        assert_eq!(stats.terrain.len(), 35);
        assert_eq!(stats.terrain[34], -34_000);
        assert_eq!(stats.river, [-30_000, i64::MIN]);
        assert_eq!(stats.fort, [1, 2]);
        assert_eq!((stats.snow, stats.night), (i64::MAX, -50_000));
    }

    #[test]
    fn movement_operands_come_from_the_same_block() {
        let mut bytes = memory(4);
        for index in 0..4 {
            put(
                &mut bytes,
                ENTRIES + index * UNIT_ADJUSTER_STRIDE + UNIT_ADJUSTER_MOVEMENT,
                &(index as i64 * 7 - 9).to_le_bytes(),
            );
        }
        for (adjuster, value) in MOVEMENT_ADJUSTERS
            .into_iter()
            .zip([-10_000_i64, 5, -25_000])
        {
            put(
                &mut bytes,
                STATS + adjuster,
                &((BASE + UNIT_ADJUSTER_VTABLE_RVA.0) as u64).to_le_bytes(),
            );
            put(
                &mut bytes,
                STATS + adjuster + UNIT_ADJUSTER_MOVEMENT,
                &value.to_le_bytes(),
            );
        }
        put(
            &mut bytes,
            STATS + MOVEMENT_FUEL_RATE,
            &123_i64.to_le_bytes(),
        );
        put(
            &mut bytes,
            STATS + MOVEMENT_AIR_DEFENSE,
            &(-4_i64).to_le_bytes(),
        );
        let view = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let stats =
            capture_attack_stats(&view, BASE, STATS, &mut Vec::new(), &mut Vec::new()).unwrap();
        assert_eq!(stats.movement_terrain, [-9, -2, 5, 12]);
        assert_eq!(stats.movement, Some([-10_000, 5, -25_000]));
        assert_eq!((stats.fuel_rate, stats.air_defense), (123, -4));
        // An embedded adjuster that does not authenticate withholds the three words
        // and keeps the rest.
        put(
            &mut bytes,
            STATS + MOVEMENT_ADJUSTERS[1],
            &0_u64.to_le_bytes(),
        );
        let view = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        let stats =
            capture_attack_stats(&view, BASE, STATS, &mut Vec::new(), &mut Vec::new()).unwrap();
        assert_eq!(stats.movement, None);
        assert_eq!(stats.movement_terrain.len(), 4);
    }

    #[test]
    fn a_foreign_entry_or_block_refuses_the_read() {
        let mut bytes = memory(3);
        put(
            &mut bytes,
            ENTRIES + UNIT_ADJUSTER_STRIDE,
            &0_u64.to_le_bytes(),
        );
        let memory_view = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        assert!(
            capture_attack_stats(&memory_view, BASE, STATS, &mut Vec::new(), &mut Vec::new())
                .is_err()
        );
        let mut bytes = memory(3);
        put(&mut bytes, STATS, &0_u64.to_le_bytes());
        let memory_view = SliceMemory {
            base: BASE,
            bytes: &bytes,
        };
        assert!(
            capture_attack_stats(&memory_view, BASE, STATS, &mut Vec::new(), &mut Vec::new())
                .is_err()
        );
    }
}

#[cfg(test)]
mod maxima_tests {
    use super::*;
    use crate::memory::{Memory, RecordLayout, RecordSources, SliceMemory};

    /// Counts guarded calls: a gather is one call, as it is live.
    struct Calls<'a>(SliceMemory<'a>, std::cell::Cell<usize>);
    impl Memory for Calls<'_> {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            self.1.set(self.1.get() + 1);
            self.0.copy(address, destination)
        }
        fn copy_records(
            &self,
            sources: RecordSources<'_>,
            layout: &RecordLayout<'_>,
            destination: &mut [u8],
        ) -> usize {
            self.1.set(self.1.get() + 1);
            self.0.copy_records(sources, layout, destination)
        }
    }

    const STATS: usize = 0x1000;

    fn block() -> Vec<u8> {
        let mut bytes = vec![0_u8; CAN_EXFILTRATE_FROM_COAST + 1];
        bytes[..8].copy_from_slice(&(VTABLE_RVA.0 as u64).to_le_bytes());
        bytes[MAXIMUM_ORGANISATION..MAXIMUM_ORGANISATION + 8]
            .copy_from_slice(&70_000_i64.to_le_bytes());
        bytes[MAXIMUM_STRENGTH..MAXIMUM_STRENGTH + 8].copy_from_slice(&50_i64.to_le_bytes());
        bytes[CAN_EXFILTRATE_FROM_COAST] = 1;
        bytes
    }

    fn maxima(bytes: &[u8]) -> (Result<DivisionMaxima, &'static str>, usize) {
        let memory = Calls(SliceMemory { base: STATS, bytes }, Default::default());
        (capture_division_maxima(&memory, 0, STATS), memory.1.get())
    }

    /// The vtable and both cached spans are one guarded call; the spans are
    /// used only once the vtable authenticates the block.
    #[test]
    fn maxima_and_their_vtable_are_one_gather() {
        assert_eq!(
            maxima(&block()),
            (
                Ok(DivisionMaxima {
                    pointer: STATS,
                    organisation: 70_000,
                    strength: 50,
                    can_exfiltrate_from_coast: 1,
                }),
                1
            )
        );
        assert_eq!(maxima(&block()).0.unwrap().effective(), (100, 70_000));
        let mut wrong = block();
        wrong[0] ^= 1;
        assert_eq!(
            maxima(&wrong).0,
            Err("division stat block identity mismatch")
        );
        assert_eq!(maxima(&[]).0, Err("division stat block identity mismatch"));
        let short = block();
        assert_eq!(
            maxima(&short[..CAN_EXFILTRATE_FROM_COAST]).0,
            Err("division maxima unreadable")
        );
    }
}
