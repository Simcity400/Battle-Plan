//! Subunit-definition cache selected as the army stat block. The i64
//! operands are established, but cache writer/thread coverage is incomplete.
//! Other inherited widths must be established before adding readers.

/// Primary vtable.
pub const VTABLE_RVA: usize = 0x0277_3ba8;
/// CSubUnitDefinition cached defense, i64/100000.
pub const DEFENSE: usize = 0xa8;
/// CSubUnitDefinition cached breakthrough, i64/100000.
pub const BREAKTHROUGH: usize = 0xb0;
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
/// Cached fixed5 weight. Each selected occurrence is independently rounded
/// before signed-i32 narrowing. Template/population counts cannot substitute.
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

/// `division_maxima` over a gather whose first field is the vtable at
/// `pointer` and whose second is the 16 bytes at `MAXIMUM_ORGANISATION`.
pub(crate) fn division_maxima_gathered<const N: usize>(
    memory: &impl crate::Memory,
    base: usize,
    pointer: usize,
    fields: &crate::memory::FieldBatch<N>,
) -> Result<(i64, i64), &'static str> {
    let vtable = base
        .checked_add(VTABLE_RVA)
        .ok_or("division stat block identity mismatch")? as u64;
    if pointer == 0 || fields.get::<u64>(memory, 0) != Some(vtable) {
        return Err("division stat block identity mismatch");
    }
    let bytes = pointer
        .checked_add(MAXIMUM_ORGANISATION)
        .and_then(|_| fields.get::<[u8; 16]>(memory, 1))
        .ok_or("division maxima unreadable")?;
    Ok(DivisionMaxima {
        pointer,
        organisation: i64::from_le_bytes(bytes[..8].try_into().unwrap()),
        strength: i64::from_le_bytes(bytes[8..].try_into().unwrap()),
        // Only the effective maxima leave this path.
        can_exfiltrate_from_coast: 0,
    }
    .effective())
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

const MAXIMA_FIELDS: [crate::memory::RecordField; 2] = [
    crate::memory::RecordField {
        source_offset: MAXIMUM_ORGANISATION,
        destination_offset: 0,
        len: 16,
    },
    crate::memory::RecordField {
        source_offset: CAN_EXFILTRATE_FROM_COAST,
        destination_offset: 16,
        len: 1,
    },
];
const MAXIMA_LAYOUT: crate::memory::RecordLayout<'static> =
    crate::memory::RecordLayout::new(&MAXIMA_FIELDS, 17).expect("valid maxima layout");

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
    let vtable = base
        .checked_add(VTABLE_RVA)
        .ok_or("division stat block identity mismatch")? as u64;
    if pointer == 0 || crate::read::<u64>(memory, pointer) != Some(vtable) {
        return Err("division stat block identity mismatch");
    }
    // One guarded copy of both cached spans, as the maxima alone were read.
    let mut bytes = [0_u8; 17];
    if memory.copy_records(
        crate::memory::RecordSources::Pointers(&[pointer]),
        &MAXIMA_LAYOUT,
        &mut bytes,
    ) < MAXIMA_FIELDS.len()
    {
        return Err("division maxima unreadable");
    }
    Ok(DivisionMaxima {
        pointer,
        organisation: i64::from_le_bytes(bytes[..8].try_into().unwrap()),
        strength: i64::from_le_bytes(bytes[8..16].try_into().unwrap()),
        can_exfiltrate_from_coast: bytes[16],
    })
}

/// Exact cached defense and soft/hard attack for demanded class-zero armies;
/// these values are not reconstructed.
pub fn capture_division_combat_stats(
    memory: &impl crate::Memory,
    base: usize,
    pointer: usize,
) -> Result<[i64; 3], &'static str> {
    let expected = base
        .checked_add(VTABLE_RVA)
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
pub const DATABASE_RVA: usize = 0x0331_6cc0;
/// The database's CPdxArray of definition pointers.
pub const DATABASE_VECTOR: usize = 0x40;

/// The global subunit database, read once per walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Database(crate::cpdx_array::Array);

impl Database {
    pub fn read(memory: &impl crate::Memory, base: usize) -> Result<Self, &'static str> {
        let database = base
            .checked_add(DATABASE_RVA)
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

/// One ordered archetype requirement of a definition (`+0x310` pool).
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

pub(crate) fn template_category(
    memory: &impl crate::Memory,
    pointer: usize,
) -> Result<i32, &'static str> {
    let bytes = pointer
        .checked_add(CATEGORY)
        .and_then(|p| crate::read::<[u8; 13]>(memory, p))
        .ok_or("subunit category unreadable")?;
    let value = if bytes[8] != 0 {
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
    };
    Ok(value)
}
/// CSubUnitDefinition first air-bucket enabled byte.
pub const AIR_BUCKET_1_ENABLED: usize = 0x5b4;
/// Airbase-type-0 value, 32-bit, read unsigned but consumed signed; scale unresolved.
pub const AIRBASE_TYPE_0_VALUE: usize = 0x600;
/// Airbase-type-1 value, 32-bit, read unsigned but consumed signed by pool capacity.
pub const AIRBASE_TYPE_1_VALUE: usize = 0x604;
/// Airbase-type-2 value, 32-bit, read unsigned but consumed signed by pool capacity.
pub const AIRBASE_TYPE_2_VALUE: usize = 0x608;
/// Airbase-type-3 value, 32-bit, read unsigned but consumed signed by pool capacity.
pub const AIRBASE_TYPE_3_VALUE: usize = 0x60c;
/// Airbase-capacity stride between signed 32-bit operands.
pub const AIRBASE_TYPE_STRIDE: usize = 4;
/// Airbase-capacity operand count.
pub const AIRBASE_TYPE_COUNT: usize = 4;
/// Air-equipment query following the first equipment requirement after the air gate.
pub const GET_AIR_EQUIPMENT_RVA: usize = 0x015a_4db0;
/// Airbase-type query returning a signed value to its caller.
pub const AIRBASE_TYPE_VALUE_RVA: usize = 0x0100_5dd0;
/// CSubUnitDefinition common-air flag mask.
pub const COMMON_AIR_MASK: u64 = 0x1f00_37fc00;
/// CSubUnitDefinition excluded-air flag mask.
pub const EXCLUDED_AIR_MASK: u64 = 0x1c00_010000;
/// CSubUnitDefinition excluded bit33.
pub const EXCLUDED_BIT_33: u64 = 1 << 33;
/// CSubUnitDefinition enable_ability string-vector data.
pub const ENABLED_ABILITY_DATA: usize = 0x658;
/// Enabled-ability string count, u32, with 0x20-byte string records.
pub const ENABLED_ABILITY_COUNT: usize = 0x664;
/// Subunit-definition constructor, also used for the null subtype.
pub const CONSTRUCTOR_RVA: usize = 0x0100_3cb0;
/// Null-subtype singleton pointer.
pub const NULL_OBJECT_RVA: usize = 0x0332_12a0;
/// CSubUnitDefinition null-subtype allocation size.
pub const NULL_OBJECT_SIZE: usize = 0x678;
/// Null-singleton getter used by default air-wing pools.
pub const GET_NULL_OBJECT_RVA: usize = 0x00b8_f770;
/// Null-subtype primary vtable.
pub const NULL_VTABLE_RVA: usize = 0x0292_d880;
/// Persistent null-subunit lexer token, checked against its loaded spelling.
pub const NULL_TOKEN: u32 = 0x165;

/// Embedded stat-block terrain-adjuster array.
pub const MOVEMENT_TERRAIN_ADJUSTERS: usize = 0x3f8;
/// Cached anti-air stat used by combat power calculation.
pub const MOVEMENT_AIR_DEFENSE: usize = 0x220;
/// Combat anti-air denominator define, signed fixed point.
pub const AIR_DEFENSE_DENOMINATOR_RVA: usize = 0x0331_de18;
/// Combat anti-air maximum-reduction define, signed fixed point.
pub const AIR_DEFENSE_REDUCTION_RVA: usize = 0x0331_dd98;

fn movement_identity(
    memory: &impl crate::Memory,
    base: usize,
    stats: usize,
) -> Result<(), &'static str> {
    if stats == 0
        || crate::read::<u64>(memory, stats) != base.checked_add(VTABLE_RVA).map(|p| p as u64)
    {
        Err("movement stat block identity mismatch")
    } else {
        Ok(())
    }
}

fn movement_leaf<T: crate::memory::Raw>(
    memory: &impl crate::Memory,
    pointer: usize,
    offset: usize,
) -> Result<T, &'static str> {
    pointer
        .checked_add(offset)
        .filter(|_| pointer != 0)
        .and_then(|address| crate::read(memory, address))
        .ok_or("movement stat operand unreadable")
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
    const TERRAIN_VTABLE: usize = 0x029a_a2b0;
    const ADJUSTER_VTABLE: usize = 0x0277_3a70;
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
            != base.checked_add(VTABLE_RVA).map(|p| p as u64)
    {
        return Err("movement stat block identity mismatch");
    }
    if read < 2 {
        return Err(UNREADABLE);
    }
    if u64::from_le_bytes(vtable)
        != base
            .checked_add(TERRAIN_VTABLE)
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
    let expected = base
        .checked_add(ADJUSTER_VTABLE)
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

/// Passive anti-air integer power series without floating point.
pub fn movement_air_defense(
    memory: &impl crate::Memory,
    base: usize,
    stats: usize,
) -> Result<i64, &'static str> {
    movement_identity(memory, base, stats)?;
    let defense = movement_leaf(memory, stats, MOVEMENT_AIR_DEFENSE)?;
    if defense == 0 {
        return Ok(0);
    }
    let denominator = movement_leaf(memory, base, AIR_DEFENSE_DENOMINATOR_RVA)?;
    let reduction = movement_leaf(memory, base, AIR_DEFENSE_REDUCTION_RVA)?;
    if let Some(result) = memory.movement_air_defense_value(defense, denominator, reduction) {
        return result;
    }
    movement_air_defense_value(defense, denominator, reduction)
}

#[cfg(test)]
mod movement_tests {
    use super::*;

    #[test]
    fn pure_air_memo_keeps_every_source_read_and_the_zero_defense_gate() {
        use std::cell::{Cell, RefCell};
        const BASE: usize = 0x140_000_000;
        const STATS: usize = 0x1000;
        struct Source {
            arguments: Cell<[i64; 3]>,
            fault: Cell<Option<usize>>,
            reads: RefCell<Vec<usize>>,
            hook_calls: RefCell<Vec<[i64; 3]>>,
            hook_result: Option<Result<i64, &'static str>>,
        }
        impl crate::Memory for Source {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.reads.borrow_mut().push(address);
                if self.fault.get() == Some(address) {
                    return false;
                }
                let arguments = self.arguments.get();
                let value = if address == STATS {
                    (BASE + VTABLE_RVA) as i64
                } else if address == STATS + MOVEMENT_AIR_DEFENSE {
                    arguments[0]
                } else if address == BASE + AIR_DEFENSE_DENOMINATOR_RVA {
                    arguments[1]
                } else if address == BASE + AIR_DEFENSE_REDUCTION_RVA {
                    arguments[2]
                } else {
                    return false;
                };
                output.copy_from_slice(&value.to_le_bytes());
                true
            }
            fn movement_air_defense_value(
                &self,
                defense: i64,
                denominator: i64,
                reduction: i64,
            ) -> Option<Result<i64, &'static str>> {
                self.hook_calls
                    .borrow_mut()
                    .push([defense, denominator, reduction]);
                self.hook_result
            }
        }
        let addresses = [
            STATS,
            STATS + MOVEMENT_AIR_DEFENSE,
            BASE + AIR_DEFENSE_DENOMINATOR_RVA,
            BASE + AIR_DEFENSE_REDUCTION_RVA,
        ];
        for hook_result in [None, Some(Ok(123)), Some(Err("pure calculation failure"))] {
            let source = Source {
                arguments: Cell::new([100_000, 100_000, 50_000]),
                fault: Cell::new(None),
                reads: RefCell::new(Vec::new()),
                hook_calls: RefCell::new(Vec::new()),
                hook_result,
            };
            for arguments in [
                [100_000, 100_000, 50_000],
                [-100_000, 100_000, 50_000],
                [100_000, -100_000, 50_000],
                [100_000, 100_000, -50_000],
            ] {
                source.arguments.set(arguments);
                source.reads.borrow_mut().clear();
                assert_eq!(
                    movement_air_defense(&source, BASE, STATS),
                    hook_result.unwrap_or_else(|| movement_air_defense_value(
                        arguments[0],
                        arguments[1],
                        arguments[2]
                    ))
                );
                assert_eq!(&*source.reads.borrow(), &addresses);
                assert_eq!(source.hook_calls.borrow().last(), Some(&arguments));
            }
            source.arguments.set([0, i64::MIN, i64::MAX]);
            source.fault.set(Some(BASE + AIR_DEFENSE_DENOMINATOR_RVA));
            source.reads.borrow_mut().clear();
            let hook_calls = source.hook_calls.borrow().len();
            assert_eq!(movement_air_defense(&source, BASE, STATS), Ok(0));
            assert_eq!(&*source.reads.borrow(), &addresses[..2]);
            assert_eq!(source.hook_calls.borrow().len(), hook_calls);
            source.arguments.set([100_000, 100_000, 50_000]);
            for (index, address) in addresses.into_iter().enumerate() {
                source.fault.set(Some(address));
                source.reads.borrow_mut().clear();
                assert!(movement_air_defense(&source, BASE, STATS).is_err());
                assert_eq!(&*source.reads.borrow(), &addresses[..=index]);
                assert_eq!(source.hook_calls.borrow().len(), hook_calls);
            }
        }
    }

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
        bytes[stats..stats + 8].copy_from_slice(&((base + VTABLE_RVA) as u64).to_le_bytes());
        bytes[stats + MOVEMENT_TERRAIN_ADJUSTERS..stats + MOVEMENT_TERRAIN_ADJUSTERS + 8]
            .copy_from_slice(&(cache as u64).to_le_bytes());
        bytes[stats + MOVEMENT_TERRAIN_ADJUSTERS + 8..stats + MOVEMENT_TERRAIN_ADJUSTERS + 12]
            .copy_from_slice(&1_u32.to_le_bytes());
        bytes[stats + MOVEMENT_TERRAIN_ADJUSTERS + 12..stats + MOVEMENT_TERRAIN_ADJUSTERS + 16]
            .copy_from_slice(&1_u32.to_le_bytes());
        bytes[terrain..terrain + 8].copy_from_slice(&((base + 0x029a_a2b0) as u64).to_le_bytes());
        bytes[terrain + 8..terrain + 12].copy_from_slice(&123_u32.to_le_bytes());
        bytes[terrain + 0x88..terrain + 0x8c].copy_from_slice(&1_i32.to_le_bytes());
        for pointer in [cache, terrain + 0x168] {
            bytes[pointer..pointer + 8]
                .copy_from_slice(&((base + 0x0277_3a70) as u64).to_le_bytes());
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
        assert_eq!(movement_air_defense(&memory, base, stats), Ok(0));
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
