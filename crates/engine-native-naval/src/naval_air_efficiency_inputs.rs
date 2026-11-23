use engine_native_memory::{Memory, cpdx_array::Array};
use engine_native::cnaval_combatant::field;
use engine_sites::Rva;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Modifier {
    pub raw: i64,
    pub present: u8,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub source: u8,
    pub pool_type_flags: u64,
    pub mission: i32,
    pub container_present: u8,
    pub base_crowding: i64,
    pub night: i64,
    pub modifiers: [Modifier; 3],
    pub modifier_metadata: [[u32; 2]; 4],
    pub feature_flags: u32,
    pub eligibility_callback: u8,
}

pub const METADATA_KEYS: [u32; 4] = [
    engine_executable::modifier_keys::CARRIER_TRAFFIC,
    engine_executable::modifier_keys::CARRIER_NIGHT_TRAFFIC,
    engine_executable::modifier_keys::SORTIE_EFFICIENCY,
    engine_executable::modifier_keys::FIGHTER_SORTIE_EFFICIENCY,
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WingKill {
    pub wing: u64,
    pub index: u32,
    pub category: u64,
    pub count: i32,
}

fn add(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("naval air source address overflow")
}
fn ptr(memory: &impl Memory, pointer: usize, offset: usize) -> Result<usize, &'static str> {
    field::<u64>(memory, pointer, offset).map(|p| p as usize)
}
fn site(base: usize, key: Rva) -> Result<usize, &'static str> {
    engine_sites::at(base, key).ok_or("naval air source site withheld")
}
fn auth(memory: &impl Memory, base: usize, pointer: usize, rva: Rva) -> Result<(), &'static str> {
    if pointer == 0 || field::<u64>(memory, pointer, 0)? != site(base, rva)? as u64 {
        return Err("naval air source identity mismatch");
    }
    Ok(())
}
fn array(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
    stride: usize,
) -> Result<Array, &'static str> {
    let header = field(memory, pointer, offset)?;
    let array = Array::from_header(header, stride).ok_or("naval air array invalid")?;
    Ok(array)
}
fn element(array: Array, index: u32, stride: usize) -> Result<usize, &'static str> {
    array
        .element(index, stride)
        .ok_or("naval air array element overflow")
}
pub fn wing(
    memory: &impl Memory,
    base: usize,
    reference: u64,
) -> Result<Option<usize>, &'static str> {
    let Some(secondary) = engine_native_parts::creference_registry::resolve(memory, base, reference)? else {
        return Ok(None);
    };
    let Some(primary) = secondary
        .checked_sub(engine_native_parts::cair_wing_layout::SECONDARY)
        .filter(|p| *p != 0)
    else {
        return Ok(None);
    };
    auth(memory, base, primary, engine_native_parts::cair_wing_layout::VTABLE_RVA)?;
    auth(
        memory,
        base,
        secondary,
        engine_native_parts::cair_wing_layout::SECONDARY_VTABLE_RVA,
    )?;
    if field::<u64>(memory, primary, engine_native_parts::cair_wing_layout::REFERENCE_TYPE)? != reference {
        return Err("naval air wing reference mismatch");
    }
    Ok(Some(primary))
}
fn modifier(memory: &impl Memory, address: usize, key: u32) -> Result<Modifier, &'static str> {
    if address == 0 {
        return Ok(Modifier::default());
    }
    let value = if let Some(value) = memory.modifier_accumulator_value(address, key) {
        value?
    } else {
        array(memory, address, 0, 16)?;
        engine_native_parts::modifier_accumulator::lookup_at(memory, address, key)?
    };
    Ok(Modifier {
        raw: value.raw,
        present: u8::from(value.present),
    })
}

pub fn read(
    memory: &impl Memory,
    base: usize,
    game: usize,
    plane: usize,
    group: usize,
    combat: usize,
) -> Result<Row, &'static str> {
    auth(memory, base, plane, engine_native::cfex_air::VTABLE_RVA)?;
    auth(memory, base, combat, engine_native::cnaval_combat::VTABLE_RVA)?;
    if ptr(memory, plane, 8)? != group {
        return Err("naval air group mismatch");
    }
    let wings = array(memory, plane, 0x40, 8)?;
    let mut row = Row::default();
    if wings.count == 0 {
        return Ok(row);
    }
    row.source = 1;
    let base_reference = field(memory, plane, 0x1c)?;
    let Some(air_base) = engine_native_parts::creference_registry::resolve(memory, base, base_reference)? else {
        return Ok(row);
    };
    auth(memory, base, air_base, engine_native_parts::cair_base_layout::VTABLE_RVA)?;
    if field::<u64>(memory, air_base, engine_native_parts::cair_base_layout::HANDLE)? != base_reference {
        return Err("naval air base reference mismatch");
    }
    row.source = 2;
    let carrier_reference = field(memory, air_base, engine_native_parts::cair_base_layout::HOST_REFERENCE)?;
    let Some(carrier) = engine_native_parts::creference_registry::resolve(memory, base, carrier_reference)?
    else {
        return Ok(row);
    };
    auth(memory, base, carrier, engine_native::cship::VTABLE_RVA)?;
    if field::<u64>(memory, carrier, engine_native::cship::HANDLE)? != carrier_reference {
        return Err("naval air carrier reference mismatch");
    }
    row.source = 3;
    let task = ptr(memory, carrier, engine_native::cship::TASK_FORCE)?;
    auth(memory, base, task, engine_native::ctask_force::VTABLE_RVA)?;
    let country_id = field::<u32>(memory, task, engine_native::cunit::ROSTER_OWNER)?;
    row.mission = field(memory, task, 0x4ec)?;
    let side = ptr(memory, group, engine_native_parts::cfex_group::SIDE)?;
    auth(memory, base, side, engine_native::cnaval_combatant::VTABLE_RVA)?;
    if ptr(memory, side, engine_native::cnaval_combatant::COMBAT)? != combat {
        return Err("naval air side combat mismatch");
    }
    for index in 0..wings.count {
        let reference = field(memory, element(wings, index, 8)?, 0)?;
        if let Some(primary) = wing(memory, base, reference)? {
            let pool = ptr(memory, primary, engine_native_parts::cair_wing_layout::POOL)?;
            auth(memory, base, pool, engine_native_parts::cair_wing_pool::VTABLE_RVA)?;
            let definition = ptr(memory, pool, engine_native_parts::cair_wing_pool::DEFINITION)?;
            auth(
                memory,
                base,
                definition,
                engine_native_parts::csub_unit_definition::VTABLE_RVA,
            )?;
            row.pool_type_flags =
                field(memory, definition, engine_native_parts::csub_unit_definition::AIR_FLAGS)?;
            break;
        }
    }
    row.night = field(memory, combat, 0xb8)?;
    let countries = engine_native::ccountry::Reader::new(memory, base, game)?;
    let country = countries.resolve(country_id)?;
    row.modifiers[0] = modifier(
        memory,
        add(country.pointer, engine_native_memory::country_identity::MODIFIER_ACCUMULATOR)?,
        engine_executable::modifier_keys::CARRIER_NIGHT_TRAFFIC,
    )?;
    let combat_weather = ptr(memory, combat, 0xb0)?;
    row.modifiers[1] = modifier(
        memory,
        if combat_weather == 0 {
            0
        } else {
            add(combat_weather, 0x10)?
        },
        engine_executable::modifier_keys::CARRIER_TRAFFIC,
    )?;
    let province = ptr(memory, combat, engine_native::cnaval_combat::PROVINCE)?;
    auth(memory, base, province, engine_native::cprovince::VTABLE_RVA)?;
    let province_id = field::<u32>(memory, province, engine_native::cprovince::ID)?;
    let weather = ptr(memory, game, engine_native::cgame_state::WEATHER_MANAGER)?;
    auth(memory, base, weather, engine_native::cweather_manager::VTABLE_RVA)?;
    let weather_provinces = array(memory, weather, 0x370, 0x330)?;
    row.modifiers[2] = modifier(
        memory,
        add(element(weather_provinces, province_id, 0x330)?, 0x280)?,
        engine_executable::modifier_keys::CARRIER_TRAFFIC,
    )?;
    let crowd_country = field::<u32>(memory, plane, 0x64)?;
    let slot = if crowd_country == 0 {
        0
    } else {
        countries.resolve(crowd_country)?.canonical_slot
    };
    let containers = array(memory, air_base, engine_native_air::cair_base::COUNTRY_CONTAINERS, 8)?;
    let container = ptr(memory, element(containers, slot, 8)?, 0)?;
    if container != 0 {
        auth(
            memory,
            base,
            container,
            engine_native_air::ccountry_air_container::VTABLE_RVA,
        )?;
        row.container_present = 1;
        row.base_crowding = field(memory, container, 0xd8)?;
    }
    use engine_native_parts::modifier_definition as md;
    let definitions = ptr(memory, site(base, md::TABLE_RVA)?, 0)?;
    if definitions == 0 {
        return Err("naval air modifier metadata absent");
    }
    for (out, key) in row.modifier_metadata.iter_mut().zip(METADATA_KEYS) {
        let at = add(definitions, key as usize * md::STRIDE)?;
        *out = [
            field(memory, at, md::FLAGS)?,
            field(memory, at, md::FEATURE_SELECTOR)?,
        ];
    }
    let callback = field::<u64>(memory, site(base, md::ELIGIBILITY_CALLBACK_RVA)?, 0)?;
    row.eligibility_callback = if callback == 0 {
        0
    } else if callback == site(base, md::ELIGIBILITY_FLAGS_GETTER_RVA)? as u64 {
        1
    } else {
        return Err("naval air modifier callback unsupported");
    };
    row.feature_flags = field(memory, site(base, md::TRADE_FEATURE_FLAGS_RVA)?, 0)?;
    Ok(row)
}

pub fn append_kills(
    memory: &impl Memory,
    base: usize,
    plane: usize,
    buffers: &mut crate::naval_member_details::DetailBuffers<'_>,
) -> Result<[usize; 2], &'static str> {
    capture_kills_buffer(
        memory,
        base,
        plane,
        buffers.plane_kills.suffix(0),
        &mut buffers.plane_kills_len,
        &mut buffers.plane_kills_required,
    )
}

fn capture_kills_buffer(
    memory: &impl Memory,
    base: usize,
    plane: usize,
    mut output: engine_native_memory::walk::Buffer<'_, WingKill>,
    used: &mut usize,
    demand: &mut usize,
) -> Result<[usize; 2], &'static str> {
    auth(memory, base, plane, engine_native::cfex_air::VTABLE_RVA)?;
    let wings = array(memory, plane, 0x40, 8)?;
    let mut sources = Vec::new();
    let mut required = 0usize;
    for index in 0..wings.count {
        let reference = field(memory, element(wings, index, 8)?, 0)?;
        if let Some(primary) = wing(memory, base, reference)? {
            let kills = array(memory, primary, 0x9f8, 16)?;
            required = required
                .checked_add(kills.count as usize)
                .ok_or("naval air kill demand overflow")?;
            if sources.capacity() == 0 {
                engine_allocation::reserve_exact(&mut sources, wings.count as usize)
                    .map_err(|_| "collector allocation failed")?;
            }
            sources.push((reference, kills));
        }
    }
    *demand = demand
        .checked_add(required)
        .ok_or("naval air kill demand overflow")?;
    let start = *used;
    let end = start
        .checked_add(required)
        .ok_or("naval air kill length overflow")?;
    if !output.ensure(end)? {
        return Err(engine_native::cfex_member::DETAIL_ARENA_CAPACITY);
    }
    let mut length = start;
    let mut bytes = [0u8; 64 * 16];
    for (reference, kills) in sources {
        for index in (0..kills.count as usize).step_by(64) {
            let count = (kills.count as usize - index).min(64);
            if !engine_native_memory::read_slice(
                memory,
                element(kills, index as u32, 16)?,
                &mut bytes[..count * 16],
            ) {
                return Err("naval air kill entries unreadable");
            }
            for (offset, entry) in bytes[..count * 16].chunks_exact(16).enumerate() {
                output[length] = WingKill {
                    wing: reference,
                    index: (index + offset) as u32,
                    category: u64::from_le_bytes(entry[..8].try_into().unwrap()),
                    count: i32::from_le_bytes(entry[8..12].try_into().unwrap()),
                };
                length += 1;
            }
        }
    }
    *used = end;
    Ok([start, required])
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_native_memory::read_census::ReadCensus;
    const PLANE: usize = 0x1000;
    const COMBAT: usize = 0x6000;
    const WING: usize = 0x4000;
    const HANDLE: u64 = (123 << 32) | 69;
    struct Fixture {
        bytes: Vec<u8>,
        root: u64,
    }
    impl Memory for Fixture {
        fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
            if address == engine_native_parts::creference_registry::LOW_ROOTS_RVA.0 + 69 * 8
                && destination.len() == 8
            {
                destination.copy_from_slice(&self.root.to_le_bytes());
                return true;
            }
            let Some(end) = address.checked_add(destination.len()) else {
                return false;
            };
            let Some(source) = self.bytes.get(address..end) else {
                return false;
            };
            destination.copy_from_slice(source);
            true
        }
    }
    impl Fixture {
        fn put(&mut self, at: usize, bytes: &[u8]) {
            self.bytes[at..at + bytes.len()].copy_from_slice(bytes);
        }
        fn q(&mut self, at: usize, value: u64) {
            self.put(at, &value.to_le_bytes());
        }
        fn d(&mut self, at: usize, value: u32) {
            self.put(at, &value.to_le_bytes());
        }
        fn array(&mut self, at: usize, data: usize, count: u32) {
            self.q(at, data as u64);
            self.d(at + 8, count);
            self.d(at + 12, count);
        }
        fn new() -> Self {
            let mut result = Self {
                bytes: vec![0; 0x8000],
                root: 0x2000,
            };
            result.q(PLANE, engine_native::cfex_air::VTABLE_RVA.0 as u64);
            result.q(COMBAT, engine_native::cnaval_combat::VTABLE_RVA.0 as u64);
            result.array(PLANE + 0x40, 0x1500, 1);
            result.q(0x1500, HANDLE);
            result.q(0x2008, 0x3000);
            result.d(0x2010, 1);
            result.d(0x2014, 7);
            result.bytes[0x2018] = 5;
            result.bytes[0x3000 + 13 * 24 + 4] = 255;
            let hash = HANDLE
                .rotate_left(32)
                .wrapping_mul(engine_native_parts::creference_registry::HASH_MULTIPLIER);
            let slot = (((hash >> 32) as u32 ^ hash as u32) & 7) as usize;
            result.bytes[0x3000 + slot * 24 + 4] = 1;
            result.q(0x3000 + slot * 24 + 8, HANDLE);
            result.q(0x3000 + slot * 24 + 16, (WING + 0x10) as u64);
            result.q(WING, engine_native_parts::cair_wing_layout::VTABLE_RVA.0 as u64);
            result.q(WING + 0x10, engine_native_parts::cair_wing_layout::SECONDARY_VTABLE_RVA.0 as u64);
            result.q(WING + 0x18, HANDLE);
            result.array(WING + 0x9f8, 0x5000, 2);
            for (index, count) in [i32::MIN, i32::MAX].into_iter().enumerate() {
                result.q(0x5000 + index * 16, 0x100000400);
                result.d(0x5008 + index * 16, count as u32);
            }
            result
        }
    }

    #[test]
    fn kills_preserve_full_wing_category_duplicates_and_signed_counts() {
        let fixture = Fixture::new();
        let census = ReadCensus::new(&fixture);
        let mut rows = vec![WingKill::default(); 2];
        let (mut used, mut required) = (0, 0);
        assert_eq!(
            capture_kills_buffer(&census, 0, PLANE, engine_native_memory::walk::Buffer::Growing(&mut rows), &mut used, &mut required),
            Ok([0, 2])
        );
        assert_eq!((used, required), (2, 2));
        assert_eq!((census.calls(), census.bytes()), (12, 154));
        for (index, count) in [i32::MIN, i32::MAX].into_iter().enumerate() {
            assert_eq!(
                rows[index],
                WingKill {
                    wing: HANDLE,
                    index: index as u32,
                    category: 0x100000400,
                    count
                }
            );
        }
    }
    #[test]
    fn wing_and_category_kill_counts_grow_past_the_old_limits() {
        for (wings, kills) in [(1, 4097), (257, 2)] {
            let mut fixture = Fixture::new();
            fixture.bytes.resize(0x20000 + kills * 16, 0);
            fixture.array(PLANE + 0x40, 0x10000, wings as u32);
            fixture.array(WING + 0x9f8, 0x20000, kills as u32);
            for index in 0..wings {
                fixture.q(0x10000 + index * 8, HANDLE);
            }
            for index in 0..kills {
                fixture.q(0x20000 + index * 16, index as u64 + 1);
                fixture.d(0x20008 + index * 16, i32::MIN as u32);
            }
            let mut rows = vec![WingKill::default(); wings * kills];
            let (mut used, mut required) = (0, 0);
            assert_eq!(
                capture_kills_buffer(&fixture, 0, PLANE, engine_native_memory::walk::Buffer::Growing(&mut rows), &mut used, &mut required),
                Ok([0, wings * kills])
            );
            assert_eq!((used, required), (wings * kills, wings * kills));
            assert_eq!(rows.last().unwrap().index, kills as u32 - 1);
            assert_eq!(rows.last().unwrap().count, i32::MIN);
        }
    }
    #[test]
    fn pressure_preserves_whole_plane_and_invalid_wing_is_a_failure() {
        let mut fixture = Fixture::new();
        let census = ReadCensus::new(&fixture);
        let (mut used, mut required) = (0, 0);
        assert_eq!(
            capture_kills_buffer(&census, 0, PLANE, engine_native_memory::walk::Buffer::Count, &mut used, &mut required),
            Err(engine_native::cfex_member::DETAIL_ARENA_CAPACITY)
        );
        assert_eq!((used, required), (0, 2));
        assert_eq!((census.calls(), census.bytes()), (11, 122));
        fixture.q(WING, 0);
        let census = ReadCensus::new(&fixture);
        assert_eq!(
            capture_kills_buffer(&census, 0, PLANE, engine_native_memory::walk::Buffer::Count, &mut 0, &mut 0),
            Err("naval air source identity mismatch")
        );
        assert_eq!((census.calls(), census.bytes()), (8, 90));

        let mut fixture = Fixture::new();
        fixture.bytes.truncate(0x5000 + 31);
        let census = ReadCensus::new(&fixture);
        let mut rows = vec![WingKill::default(); 2];
        let (mut used, mut required) = (0, 0);
        assert_eq!(
            capture_kills_buffer(&census, 0, PLANE, engine_native_memory::walk::Buffer::Growing(&mut rows), &mut used, &mut required),
            Err("naval air kill entries unreadable")
        );
        assert_eq!((used, required), (0, 2));
        assert_eq!(rows, [WingKill::default(); 2]);
        assert_eq!((census.calls(), census.bytes()), (12, 122));
    }
    #[test]
    fn no_active_wings_and_no_base_are_distinct_native_branches() {
        let mut fixture = Fixture::new();
        let census = ReadCensus::new(&fixture);
        assert_eq!(read(&census, 0, 0, PLANE, 0, COMBAT).unwrap().source, 1);
        assert_eq!((census.calls(), census.bytes()), (5, 48));
        fixture.array(PLANE + 0x40, 0, 0);
        let census = ReadCensus::new(&fixture);
        assert_eq!(read(&census, 0, 0, PLANE, 0, COMBAT), Ok(Row::default()));
        assert_eq!((census.calls(), census.bytes()), (4, 40));
        fixture.q(PLANE, 0);
        let census = ReadCensus::new(&fixture);
        assert_eq!(
            read(&census, 0, 0, PLANE, 0, COMBAT),
            Err("naval air source identity mismatch")
        );
        assert_eq!((census.calls(), census.bytes()), (1, 8));
    }
    #[test]
    fn modifiers_preserve_raw_values_and_absence_before_ui_clamping() {
        let mut fixture = Fixture::new();
        fixture.array(0x7000, 0x7020, 1);
        fixture.d(0x7020, 0x22);
        fixture.q(0x7028, i64::MIN as u64);
        assert_eq!(
            modifier(&fixture, 0x7000, 0x22),
            Ok(Modifier {
                raw: i64::MIN,
                present: 1
            })
        );
        assert_eq!(modifier(&fixture, 0x7000, 0x23), Ok(Modifier::default()));
        fixture.array(0x7000, 0x7020, 8193);
        assert!(modifier(&fixture, 0x7000, 0x22).is_err());
    }
}
