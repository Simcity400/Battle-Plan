
use engine_sites::Rva;
pub const VTABLE_RVA: Rva = Rva(0x0280_7710);
pub const DEFENSE: usize = 0xa8;
pub const BREAKTHROUGH: usize = 0xb0;
pub const COMBAT_WIDTH: usize = 0x58;
pub const HARDNESS: usize = 0xb8;
pub const SOFT_ATTACK: usize = 0xc0;
pub const HARD_ATTACK: usize = 0xc8;
pub const MAXIMUM_ORGANISATION: usize = 0x280;
pub const MAXIMUM_STRENGTH: usize = 0x288;
pub const MAXIMUM_SPEED: usize = 0x290;
pub const TRANSPORT_WEIGHT: usize = 0x2b8;
pub const CAN_EXFILTRATE_FROM_COAST: usize = 0x5c0;

pub fn division_maxima(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    pointer: usize,
) -> Result<(i64, i64), &'static str> {
    Ok(capture_division_maxima(memory, base, pointer)?.effective())
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DivisionMaxima {
    pub pointer: usize,
    pub organisation: i64,
    pub strength: i64,
    pub can_exfiltrate_from_coast: u8,
}

const MAXIMA_FIELDS: [engine_native_memory::memory::RecordField; 3] = [
    engine_native_memory::memory::RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    engine_native_memory::memory::RecordField {
        source_offset: MAXIMUM_ORGANISATION,
        destination_offset: 8,
        len: 16,
    },
    engine_native_memory::memory::RecordField {
        source_offset: CAN_EXFILTRATE_FROM_COAST,
        destination_offset: 24,
        len: 1,
    },
];
const MAXIMA_LAYOUT: engine_native_memory::memory::RecordLayout<'static> =
    engine_native_memory::memory::RecordLayout::new(&MAXIMA_FIELDS, 25).expect("valid maxima layout");

pub struct MaximaBatch {
    bytes: [u8; 64 * 25],
    complete: [bool; 64],
    len: usize,
}

impl MaximaBatch {
    pub fn new() -> Self {
        Self {
            bytes: [0; 64 * 25],
            complete: [false; 64],
            len: 0,
        }
    }

    pub fn capture(&mut self, memory: &impl engine_native_memory::Memory, pointers: &[usize]) {
        self.len = pointers.len();
        engine_native_memory::memory::gather_records_once(
            memory,
            pointers,
            &MAXIMA_LAYOUT,
            &mut self.bytes[..self.len * 25],
            &mut self.complete,
            engine_native_memory::batch_fallback::Batch::DivisionMaxima,
        );
    }

    pub fn read(
        &self,
        memory: &impl engine_native_memory::Memory,
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

pub fn stage_division_maxima(
    memory: &impl engine_native_memory::Memory,
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
    memory: &impl engine_native_memory::Memory,
    base: usize,
    pointer: usize,
) -> Result<DivisionMaxima, &'static str> {
    engine_sites::at(base, VTABLE_RVA)
        .ok_or("division stat block identity mismatch")?;
    if pointer == 0 {
        return Err("division stat block identity mismatch");
    }
    let mut bytes = [0_u8; 25];
    let copied = memory.copy_records(
        engine_native_memory::memory::RecordSources::Pointers(&[pointer]),
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

pub fn capture_division_combat_stats(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    pointer: usize,
) -> Result<[i64; 3], &'static str> {
    let expected = engine_sites::at(base, VTABLE_RVA)
        .ok_or("division combat stat module overflow")? as u64;
    if pointer == 0 || engine_native_memory::read::<u64>(memory, pointer) != Some(expected) {
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
pub const BASE_FUEL: usize = 0x2c8;
pub const TOKEN: usize = 0x08;
pub const IS_REAL: usize = 0x10;
pub const NAME: usize = 0x18;
pub const ARCHETYPE_REQUIREMENTS: usize = 0x310;
pub const ARCHETYPE_REQUIREMENTS_DATA: usize = 0x318;
pub const ARCHETYPE_REQUIREMENT_STRIDE: usize = 0x10;
pub const DATABASE_INDEX: usize = 0x58c;
pub const DATABASE_RVA: Rva = Rva(0x033d_d4d8);
pub const DATABASE_VECTOR: usize = 0x40;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Database(engine_native_memory::cpdx_array::Array);

impl Database {
    pub fn read(memory: &impl engine_native_memory::Memory, base: usize) -> Result<Self, &'static str> {
        let database = engine_sites::at(base, DATABASE_RVA)
            .and_then(|at| engine_native_memory::read::<u64>(memory, at))
            .and_then(|at| usize::try_from(at).ok())
            .filter(|at| *at != 0)
            .ok_or("subunit database pointer unreadable or null")?;
        database
            .checked_add(DATABASE_VECTOR)
            .and_then(|at| engine_native_memory::cpdx_array::Array::read(memory, at, 8))
            .filter(|vector| vector.count > 0)
            .map(Self)
            .ok_or("subunit database vector invalid")
    }

    pub fn vector(self) -> engine_native_memory::cpdx_array::Array {
        self.0
    }

    pub fn entry(self, memory: &impl engine_native_memory::Memory, index: i32) -> Result<usize, &'static str> {
        u32::try_from(index)
            .ok()
            .filter(|index| *index != 0)
            .and_then(|index| self.0.element(index, 8))
            .and_then(|at| engine_native_memory::read::<u64>(memory, at))
            .and_then(|global| usize::try_from(global).ok())
            .filter(|global| *global != 0)
            .ok_or("subunit database index outside the database")
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Requirement {
    pub archetype: usize,
    pub amount: i64,
}

pub fn requirements(
    memory: &impl engine_native_memory::Memory,
    definition: usize,
) -> Result<engine_native_memory::cpdx_array::Array, &'static str> {
    definition
        .checked_add(ARCHETYPE_REQUIREMENTS_DATA)
        .and_then(|at| engine_native_memory::cpdx_array::Array::read(memory, at, ARCHETYPE_REQUIREMENT_STRIDE))
        .ok_or("subunit requirement vector invalid")
}

pub fn requirement(
    memory: &impl engine_native_memory::Memory,
    vector: engine_native_memory::cpdx_array::Array,
    index: u32,
) -> Result<Requirement, &'static str> {
    let bytes = vector
        .element(index, ARCHETYPE_REQUIREMENT_STRIDE)
        .and_then(|at| engine_native_memory::read::<[u8; 16]>(memory, at))
        .ok_or("subunit requirement unreadable")?;
    Ok(Requirement {
        archetype: usize::try_from(u64::from_le_bytes(bytes[..8].try_into().unwrap()))
            .ok()
            .filter(|at| *at != 0)
            .ok_or("subunit requirement archetype null")?,
        amount: i64::from_le_bytes(bytes[8..].try_into().unwrap()),
    })
}
pub const AIR_FLAGS: usize = 0x5a8;
pub const CATEGORY: usize = 0x5b0;
pub const SPEED_MODIFIER_KEY: usize = 0x5d8;
pub const CATEGORIES: usize = 0x508;
pub const CATEGORY_MODIFIER_KEY: usize = 0x60;

pub fn template_category(
    memory: &impl engine_native_memory::Memory,
    pointer: usize,
) -> Result<i32, &'static str> {
    let bytes = pointer
        .checked_add(CATEGORY)
        .and_then(|p| engine_native_memory::read::<[u8; CATEGORY_LEN]>(memory, p))
        .ok_or("subunit category unreadable")?;
    Ok(category_of(&bytes))
}

pub const CATEGORY_LEN: usize = 13;

pub fn category_of(bytes: &[u8; CATEGORY_LEN]) -> i32 {
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
pub const CONSTRUCTOR_RVA: Rva = Rva(0x0109_ee10);
pub const NULL_OBJECT_RVA: Rva = Rva(0x033e_7be0);
pub const GET_NULL_OBJECT_RVA: Rva = Rva(0x00bf_1d50);
pub const NULL_VTABLE_RVA: Rva = Rva(0x029c_1178);
pub const NULL_TOKEN: u32 = 0x165;

pub const MOVEMENT_TERRAIN_ADJUSTERS: usize = 0x3f8;
pub const MOVEMENT_AIR_DEFENSE: usize = 0x220;
pub const AIR_DEFENSE_DENOMINATOR_RVA: Rva = Rva(0x033e_18b8);
pub const AIR_DEFENSE_REDUCTION_RVA: Rva = Rva(0x033e_1830);

fn movement_identity(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    stats: usize,
) -> Result<(), &'static str> {
    if stats == 0
        || engine_native_memory::read::<u64>(memory, stats) != engine_sites::at(base, VTABLE_RVA).map(|p| p as u64)
    {
        Err("movement stat block identity mismatch")
    } else {
        Ok(())
    }
}

pub fn movement_terrain(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    stats: usize,
    terrain: usize,
) -> Result<i64, &'static str> {
    const TERRAIN_VTABLE: Rva = Rva(0x02a3_b470);
    const ADJUSTER_VTABLE: Rva = Rva(0x0280_75d8);
    const UNREADABLE: &str = "movement stat operand unreadable";
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
    let index = i32::from_le_bytes(index);
    array_at.ok_or("terrain array overflow")?;
    let array = (read > 3)
        .then(|| engine_native_memory::cpdx_array::Array::from_header(header, 0x28))
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

pub fn movement_air_defense_value(
    defense: i64,
    denominator: i64,
    reduction: i64,
) -> Result<i64, &'static str> {
    if defense == 0 {
        return Ok(0);
    }
    let power = engine_native_memory::fixed5::power(defense, 150_000)?;
    let ratio = engine_native_memory::fixed5::fraction(power, power.wrapping_add(denominator))?;
    Ok(ratio.wrapping_mul(reduction) / engine_native_memory::fixed5::SCALE)
}

#[cfg(test)]
mod movement_tests {
    use super::*;

    #[test]
    fn integer_series_exact_identity_and_zero() {
        assert_eq!(engine_native_memory::fixed5::log(100_000), Ok(0));
        assert_eq!(engine_native_memory::fixed5::exp(0), Ok(100_000));
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
        assert_eq!(engine_native_memory::fixed5::fraction(1, 0), Ok(0xffff_ffff));
        assert_eq!(engine_native_memory::fixed5::fraction(i64::MIN, 100_000), Ok(0xffff_ffff));
        assert_eq!(engine_native_memory::fixed5::fraction(i64::MAX, 100_000), Ok(0xffff_ffff));
        let value = i64::MAX / 2;
        assert_eq!(
            engine_native_memory::fixed5::fraction(value, 100_000),
            Ok(value.wrapping_mul(100_000) / 100_000)
        );
    }

    #[test]
    fn fractional_power_preserves_native_series_rounding() {
        assert_eq!(engine_native_memory::fixed5::log(50_000), Ok(-69_308));
        assert_eq!(engine_native_memory::fixed5::log(1_000_000), Ok(230_258));
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
        use engine_native_memory::memory::SliceMemory;
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

pub const UNIT_ADJUSTER_VTABLE_RVA: Rva = Rva(0x0280_75d8);
pub const UNIT_ADJUSTER_STRIDE: usize = 0x28;
pub const UNIT_ADJUSTER_ATTACK: usize = 0x10;
pub const ATTACK_NIGHT: usize = 0x340;
pub const ATTACK_FORT: usize = 0x368;
pub const ATTACK_RIVER: usize = 0x390;
pub const ATTACK_SNOW: usize = 0x3e0;
const ATTACK_SPAN: usize = MOVEMENT_TERRAIN_ADJUSTERS + 16 - ATTACK_NIGHT;
pub const UNIT_ADJUSTER_MOVEMENT: usize = 0x20;
pub const MOVEMENT_ADJUSTERS: [usize; 3] = [0x358, 0x380, 0x3d0];
pub const MOVEMENT_FUEL_RATE: usize = 0x120;

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

pub const SUPPRESSION_CACHE_FIELD: (u8, usize) = (0xc, 0x100);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AttackStats {
    pub panel: [Option<i64>; 23],
    pub suppression: Option<i64>,
    pub terrain: Vec<i64>,
    pub river: [i64; 2],
    pub fort: [i64; 2],
    pub snow: i64,
    pub night: i64,
    pub movement_terrain: Vec<i64>,
    pub movement: Option<[i64; 3]>,
    pub fuel_rate: i64,
    pub air_defense: i64,
}

pub fn capture_attack_stats(
    memory: &impl engine_native_memory::Memory,
    base: usize,
    stats: usize,
    terrain: &mut Vec<i64>,
    movement_terrain: &mut Vec<i64>,
) -> Result<AttackStats, &'static str> {
    capture_attack_stats_holding(memory, base, stats, terrain, movement_terrain, None)
}

pub fn capture_attack_stats_holding(
    memory: &impl engine_native_memory::Memory,
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
    if !engine_native_memory::read_slice(memory, at, &mut span) {
        return Err("stat block attack operands unreadable");
    }
    let word = |offset: usize| {
        let at = offset - ATTACK_NIGHT;
        i64::from_le_bytes(span[at..at + 8].try_into().unwrap())
    };
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
    let array = engine_native_memory::cpdx_array::Array::from_header(
        span[header..header + 16].try_into().unwrap(),
        UNIT_ADJUSTER_STRIDE,
    )
    .ok_or("stat block terrain adjuster array invalid")?;
    let expected = engine_sites::at(base, UNIT_ADJUSTER_VTABLE_RVA)
        .ok_or("module address overflow")? as u64;
    terrain.clear();
    movement_terrain.clear();
    engine_allocation::reserve_exact(terrain, array.count as usize)
        .map_err(|_| "stat block terrain allocation failed")?;
    engine_allocation::reserve_exact(movement_terrain, array.count as usize)
        .map_err(|_| "stat block terrain allocation failed")?;
    const BATCH: usize = 32;
    let mut entries = [0_u8; BATCH * UNIT_ADJUSTER_STRIDE];
    let mut first = 0;
    while first < array.count {
        let len = (array.count - first).min(BATCH as u32) as usize;
        let address = array
            .element(first, UNIT_ADJUSTER_STRIDE)
            .ok_or("stat block terrain adjuster address overflow")?;
        if !engine_native_memory::read_slice(memory, address, &mut entries[..len * UNIT_ADJUSTER_STRIDE]) {
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
    use engine_native_memory::memory::SliceMemory;

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

    #[test]
    fn attack_operands_read_every_entry_and_exact_scalars() {
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
    use engine_native_memory::memory::{Memory, RecordLayout, RecordSources, SliceMemory};

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
