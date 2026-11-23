use super::*;
use crate::memory::{RecordField, RecordLayout, RecordSources};
use crate::modifier_keys as keys;
use crate::{
    cair_base,
    cair_wing_pool,
    cbuilding,
    cbuilding_database,
    cprovince::{buildings as cbuilding_status, self},
    cbuilding_template,
    cgame_state,
    cstate,
    cstrategic_air,
    cstrategic_air_manager,
    cstrategic_region,
};
use engine_sites::Rva;

pub(super) fn authenticate(
    m: &impl Memory,
    base: usize,
    p: usize,
    vt: Rva,
) -> Result<(), &'static str> {
    if Some(field::<u64>(m, p, 0)?) != engine_sites::at(base, vt).map(|a| a as u64) {
        return Err("raid tooltip context vtable mismatch");
    }
    Ok(())
}
fn pointer(m: &impl Memory, p: usize, o: usize) -> Result<usize, &'static str> {
    Ok(field::<u64>(m, p, o)? as usize)
}
fn array(m: &impl Memory, p: usize, o: usize, stride: usize) -> Result<Array, &'static str> {
    p.checked_add(o)
        .and_then(|p| Array::read(m, p, stride))
        .ok_or("raid tooltip context array invalid")
}
fn item(m: &impl Memory, a: Array, i: u32) -> Result<usize, &'static str> {
    pointer(
        m,
        a.element(i, 8)
            .ok_or("raid tooltip context index out of bounds")?,
        0,
    )
}
fn region(m: &impl Memory, base: usize, province: usize) -> Result<Option<u32>, &'static str> {
    if province == 0 {
        return Ok(None);
    }
    authenticate(m, base, province, cprovince::VTABLE_RVA)?;
    region_id(m, base, pointer(m, province, cprovince::STRATEGIC_REGION)?)
}
fn region_id(m: &impl Memory, base: usize, p: usize) -> Result<Option<u32>, &'static str> {
    cstrategic_region::id(m, base, p)
}

pub(super) fn source(
    m: &impl Memory,
    base: usize,
    game: usize,
    raid: usize,
    sink: &mut Sink<'_>,
) -> Result<(), &'static str> {
    let source = raid + craid_instance::SOURCE;
    if field::<u8>(m, source, 4)? == 0 {
        return Ok(());
    }
    let airbase = match field::<u32>(m, source, 0)? {
        0 => {
            let countries = crate::ccountry::Reader::new(m, base, game)?;
            let country = countries.resolve(field(m, source, 0x18)?)?;
            let manager = pointer(m, game, cgame_state::STRATEGIC_AIR_MANAGER)?;
            authenticate(m, base, manager, cstrategic_air_manager::VTABLE_RVA)?;
            let air = item(
                m,
                array(m, manager, cstrategic_air_manager::COUNTRY_AIR, 8)?,
                country.canonical_slot,
            )?;
            authenticate(m, base, air, cstrategic_air::VTABLE_RVA)?;
            let province = pointer(m, source, 8)?;
            authenticate(m, base, province, cprovince::VTABLE_RVA)?;
            let state = pointer(m, province, cprovince::STATE)?;
            authenticate(m, base, state, cstate::VTABLE_RVA)?;
            let access = pointer(m, air, 0x88)?;
            item(m, array(m, access, 0x90, 8)?, field(m, state, cstate::ID)?)?
        }
        3 | 4 => match creference_registry::resolve(m, base, field(m, source, 0x10)?)? {
            None => 0,
            Some(ship) => {
                authenticate(m, base, ship, crate::cship::VTABLE_RVA)?;
                pointer(m, ship, 0x730)?
            }
        },
        _ => return Ok(()),
    };
    let kind = if airbase == 0 {
        -1
    } else {
        authenticate(m, base, airbase, cair_base::VTABLE_RVA)?;
        i64::from(field::<i32>(m, airbase, cair_base::KIND)?)
    };
    sink.push(NAME, SOURCE_AIR_BASE_KIND, kind)
}

pub(super) fn target(
    m: &impl Memory,
    base: usize,
    game: usize,
    raid: usize,
    state: usize,
    province: usize,
    sink: &mut Sink<'_>,
) -> Result<(), &'static str> {
    sink.push(
        CONTEXT,
        TARGET_COUNTRY,
        i64::from(target_country(m, base, game, raid)?),
    )?;
    if province != 0 {
        sink.push(
            CONTEXT,
            TARGET_PROVINCE,
            i64::from(field::<u32>(m, province, cprovince::ID)?),
        )?;
        let province_state = pointer(m, province, cprovince::STATE)?;
        if province_state != 0 {
            authenticate(m, base, province_state, cstate::VTABLE_RVA)?;
            let cores = array(m, province_state, 0xb0, 4)?;
            for index in 0..cores.count {
                sink.push(
                    TARGET_CORES,
                    index,
                    i64::from(field::<u32>(
                        m,
                        cores
                            .element(index, 4)
                            .ok_or("raid tooltip core index overflow")?,
                        0,
                    )?),
                )?;
            }
        }
    }
    if state == 0 {
        return Ok(());
    }
    let provinces = array(m, state, cstate::PROVINCES, 8)?;
    let first = if provinces.count == 0 {
        0
    } else {
        item(m, provinces, 0)?
    };
    if first != 0 {
        authenticate(m, base, first, cprovince::VTABLE_RVA)?;
        let region = pointer(m, first, cprovince::STRATEGIC_REGION)?;
        if let Some(id) = region_id(m, base, region)? {
            sink.push(CONTEXT, TARGET_REGION, i64::from(id))?;
            interception(m, base, game, raid, (region, id), sink)?;
        }
    }
    anti_air(m, base, state, sink)
}

fn target_country(
    m: &impl Memory,
    base: usize,
    game: usize,
    raid: usize,
) -> Result<u32, &'static str> {
    let s = crate::craid_target::read_row(m, base, raid + craid_instance::TARGET)?;
    if s.building_type != 0 {
        if s.building_country > 0 {
            return Ok(s.building_country as u32);
        }
        let provincial = s.building_flags & 1 != 0;
        let p = object(m, base, game, s.building_location, provincial)?;
        return field(
            m,
            p,
            if provincial {
                cprovince::CONTROLLER
            } else {
                cstate::CONTROLLER
            },
        );
    }
    if s.province != 0 {
        return field(
            m,
            object(m, base, game, s.province, true)?,
            cprovince::CONTROLLER,
        );
    }
    if s.state != 0 {
        return field(
            m,
            object(m, base, game, s.state, false)?,
            cstate::CONTROLLER,
        );
    }
    if let Some(country) = s.leader_country {
        return Ok(country as u32);
    }
    if s.leader_refused {
        return Err("raid tooltip target captor unsupported");
    }
    if s.leader_province != 0 {
        return field(
            m,
            object(m, base, game, s.leader_province, true)?,
            cprovince::CONTROLLER,
        );
    }
    Ok(0)
}

fn interception(
    m: &impl Memory,
    base: usize,
    game: usize,
    raid: usize,
    region: (usize, u32),
    sink: &mut Sink<'_>,
) -> Result<(), &'static str> {
    let countries = crate::ccountry::Reader::new(m, base, game)?;
    let raw = target_country(m, base, game, raid)?;
    let Some(slot) = countries.link(raw)? else {
        return Ok(());
    };
    let manager = pointer(m, game, cgame_state::STRATEGIC_AIR_MANAGER)?;
    authenticate(m, base, manager, cstrategic_air_manager::VTABLE_RVA)?;
    let owners = array(m, manager, cstrategic_air_manager::COUNTRY_AIR, 8)?;
    let owner = item(m, owners, slot)?;
    if owner == 0 {
        return Ok(());
    }
    authenticate(m, base, owner, cstrategic_air::VTABLE_RVA)?;
    if countries.link(field(m, owner, cstrategic_air::COUNTRY)?)? != Some(slot) {
        return Err("raid tooltip air owner mismatch");
    }
    interceptors(
        m,
        base,
        array(m, owner, cstrategic_air::POOLS, 8)?,
        region,
        sink,
    )
}

fn interceptors(
    m: &impl Memory,
    base: usize,
    pools: Array,
    (region, target): (usize, u32),
    sink: &mut Sink<'_>,
) -> Result<(), &'static str> {
    const BATCH: usize = 64;
    const SCREEN: [RecordField; 2] = [
        RecordField {
            source_offset: 0,
            destination_offset: 0,
            len: 8,
        },
        RecordField {
            source_offset: 0xc0,
            destination_offset: 8,
            len: 8,
        },
    ];
    const LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&SCREEN, 16).expect("valid wing screen layout");
    let wing_vtable =
        engine_sites::at(base, cair_wing::VTABLE_RVA).ok_or("raid tooltip vtable withheld")? as u64;
    let mut ordinal = 0u32;
    let mut raw = [0u8; BATCH * 8];
    let mut pointers = [0usize; BATCH];
    let mut screened = [0u8; BATCH * 16];
    for i in 0..pools.count {
        let pool = item(m, pools, i)?;
        authenticate(m, base, pool, cair_wing_pool::VTABLE_RVA)?;
        let wings = array(m, pool, cair_wing_pool::WINGS, 8)?;
        for first in (0..wings.count).step_by(BATCH) {
            let count = ((wings.count - first) as usize).min(BATCH);
            let at = wings
                .element(first, 8)
                .ok_or("raid tooltip context index out of bounds")?;
            if !m.copy(at, &mut raw[..count * 8]) {
                return Err("raid tooltip wing pointers unreadable");
            }
            for (pointer, bytes) in pointers.iter_mut().zip(raw[..count * 8].chunks_exact(8)) {
                *pointer = u64::from_le_bytes(bytes.try_into().unwrap()) as usize;
            }
            if m.copy_records(
                RecordSources::Pointers(&pointers[..count]),
                &LAYOUT,
                &mut screened[..count * 16],
            ) != count * SCREEN.len()
            {
                return Err("raid tooltip operand unreadable");
            }
            for (&wing, record) in pointers[..count].iter().zip(screened.chunks_exact(16)) {
                if u64::from_le_bytes(record[..8].try_into().unwrap()) != wing_vtable {
                    return Err("raid tooltip context vtable mismatch");
                }
                if u64::from_le_bytes(record[8..].try_into().unwrap()) as usize != region {
                    continue;
                }
                interceptor(m, base, wing, target, &mut ordinal, sink)?;
            }
        }
    }
    Ok(())
}

fn interceptor(
    m: &impl Memory,
    base: usize,
    wing: usize,
    target: u32,
    ordinal: &mut u32,
    sink: &mut Sink<'_>,
) -> Result<(), &'static str> {
    let associated = pointer(m, wing, 0x90 + crate::air_mission::WING)?;
    authenticate(m, base, associated, cair_wing::VTABLE_RVA)?;
    sink.push(
        INTERCEPTION_ENABLED,
        *ordinal,
        i64::from(field::<u32>(m, wing, 0xa0)?),
    )?;
    sink.push(
        INTERCEPTION_SUPPORTED,
        *ordinal,
        i64::from(field::<u32>(m, associated, 0x30)?),
    )?;
    sink.push(INTERCEPTION_REGION, *ordinal, i64::from(target))?;
    sink.push(
        INTERCEPTION_PLANES,
        *ordinal,
        i64::from(field::<i32>(m, wing, cair_wing::PLANE_COUNT)?),
    )?;
    *ordinal = ordinal
        .checked_add(1)
        .ok_or("raid tooltip interception ordinal overflow")?;
    Ok(())
}

fn anti_air(
    m: &impl Memory,
    base: usize,
    state: usize,
    sink: &mut Sink<'_>,
) -> Result<(), &'static str> {
    let db = engine_sites::at(base, cbuilding_database::SINGLETON_RVA)
        .ok_or("raid tooltip building database withheld")?;
    let db = pointer(m, db, 0)?;
    let types = array(m, db, 0x220, 8)?;
    let status = state
        .checked_add(cstate::BUILDING_STATUS)
        .ok_or("raid tooltip building status overflow")?;
    authenticate(m, base, status, cbuilding_status::VTABLE_RVA)?;
    let indices = array(m, status, cbuilding_status::BUILDING_INDICES, 4)?;
    if indices.count == 0 {
        return Ok(());
    }
    let buildings = array(m, status, cbuilding_status::BUILDINGS, 8)?;
    for i in 0..types.count {
        let kind = item(m, types, i)?;
        if field::<u8>(m, kind, cbuilding_template::LOOKUP_ENABLED_CANDIDATE)? == 0 {
            continue;
        }
        let ordinal = field::<u32>(m, kind, cbuilding_template::DATABASE_ORDINAL)?;
        let at = indices
            .element(ordinal, 4)
            .ok_or("raid tooltip building ordinal outside index")?;
        let index = field::<i32>(m, at, 0)?;
        if index == -1 {
            continue;
        }
        let building = item(
            m,
            buildings,
            u32::try_from(index).map_err(|_| "raid tooltip negative building index")?,
        )?;
        authenticate(m, base, building, cbuilding::VTABLE_RVA)?;
        if pointer(m, building, cbuilding::TEMPLATE)? != kind {
            return Err("raid tooltip building template mismatch");
        }
        sink.push(
            ANTI_AIR_LEVELS,
            field(m, kind, cbuilding_template::TYPE_ID)?,
            i64::from(field::<i16>(m, building, cbuilding::HEALTHY_LEVEL)?),
        )?;
    }
    Ok(())
}

pub(super) fn wing(
    m: &impl Memory,
    base: usize,
    game: usize,
    wing: usize,
    province: usize,
    sink: &mut Sink<'_>,
) -> Result<(), &'static str> {
    let mission = region_id(m, base, pointer(m, wing, 0xc0)?)?;
    sink.push(
        CONTEXT,
        WING_MISSION_MASK,
        i64::from(field::<u32>(m, wing, 0xa0)?),
    )?;
    let Some(mission) = mission else {
        return Ok(());
    };
    sink.push(CONTEXT, WING_REGION, i64::from(mission))?;
    if province == 0 {
        return Ok(());
    }
    let id = field::<u32>(m, province, cprovince::ID)?;
    if id == 0 {
        return Ok(());
    }
    let manager = pointer(m, game, cgame_state::WEATHER_MANAGER)?;
    authenticate(m, base, manager, crate::cweather_manager::VTABLE_RVA)?;
    let weather = array(m, manager, crate::cweather_manager::REGIONS, 0x160)?
        .element(mission, 0x160)
        .ok_or("raid tooltip weather region outside array")?;
    modifiers(
        m,
        weather + 0x58,
        WEATHER_MODIFIERS,
        &[
            keys::AIR_STRATEGIC_BOMBER_BOMBING_FACTOR,
            keys::AIR_DEFENCE_FACTOR,
            keys::AIR_AGILITY_FACTOR,
        ],
        sink,
    )?;
    let environment = array(m, manager, 0x370, 0x330)?
        .element(id, 0x330)
        .ok_or("raid tooltip weather province outside array")?;
    sink.push(CONTEXT, NIGHT_FACTOR, field(m, environment, 0x240)?)?;
    modifiers(
        m,
        environment + 0x280,
        NIGHT_MODIFIERS,
        &[
            keys::AIR_STRATEGIC_BOMBER_BOMBING_FACTOR,
            keys::AIR_DEFENCE_FACTOR,
            keys::AIR_AGILITY_FACTOR,
        ],
        sink,
    )?;
    let pool = pointer(m, wing, cair_wing::POOL)?;
    authenticate(m, base, pool, cair_wing_pool::VTABLE_RVA)?;
    let base_ref = field::<u64>(m, pool, cair_wing_pool::AIR_BASE_REFERENCE)?;
    if let Some(airbase) = creference_registry::resolve(m, base, base_ref)? {
        authenticate(m, base, airbase, cair_base::VTABLE_RVA)?;
        let state = pointer(m, airbase, cair_base::STATE)?;
        let host = field::<u64>(m, airbase, cair_base::HOST_REFERENCE)?;
        if state != 0
            && let Some(id) = region(m, base, land_base_province(m, base, game, airbase, state)?)?
        {
            sink.push(CONTEXT, BASE_REGION, i64::from(id))?;
        }
        if let Some(ship) = creference_registry::resolve(m, base, host)? {
            authenticate(m, base, ship, crate::cship::VTABLE_RVA)?;
            let task = pointer(m, ship, 0x728)?;
            authenticate(m, base, task, crate::ctask_force::VTABLE_RVA)?;
            sink.push(
                CONTEXT,
                CARRIER_MISSION,
                i64::from(field::<i32>(m, task, 0x374)?),
            )?;
            if state == 0 {
                let mut location = pointer(m, task, 0x1f0)?;
                let definition = pointer(m, location, cprovince::DEFINITION)?;
                if field::<u8>(m, definition, 0xd2)? & 1 != 0 {
                    location = object(m, base, game, field(m, definition, 0xc8)?, true)?;
                }
                if let Some(id) = region(m, base, location)? {
                    sink.push(CONTEXT, BASE_REGION, i64::from(id))?;
                }
            }
        }
    }
    Ok(())
}

fn land_base_province(
    m: &impl Memory,
    base: usize,
    game: usize,
    airbase: usize,
    state: usize,
) -> Result<usize, &'static str> {
    let cached = pointer(m, airbase, cair_base::PROVINCE)?;
    if cached != 0 {
        return Ok(cached);
    }
    authenticate(m, base, state, cstate::VTABLE_RVA)?;
    let kind = field::<i32>(m, airbase, cair_base::KIND)?;
    let offset = match kind {
        1 => 0x60,
        2 => 0x78,
        _ => 0x48,
    };
    let manager = pointer(m, game, cgame_state::STRATEGIC_AIR_MANAGER)?;
    authenticate(m, base, manager, cstrategic_air_manager::VTABLE_RVA)?;
    let id = field::<u32>(m, state, cstate::ID)?;
    let at = array(m, manager, offset, 24)?
        .element(id, 24)
        .ok_or("raid tooltip airbase state index invalid")?;
    let provinces = array(m, at, 0, 4)?;
    if provinces.count == 0 {
        return Ok(0);
    }
    object(
        m,
        base,
        game,
        field(m, provinces.element(0, 4).unwrap(), 0)?,
        true,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    const REGION: usize = 0x9000;

    fn world() -> Vec<u8> {
        let mut bytes = vec![0u8; 0x40_000];
        let mut put =
            |at: usize, value: u64| bytes[at..at + 8].copy_from_slice(&value.to_le_bytes());
        put(0x100, 0x200);
        put(0x200, cair_wing_pool::VTABLE_RVA.0 as u64);
        put(0x200 + cair_wing_pool::WINGS, 0x400);
        put(0x200 + cair_wing_pool::WINGS + 8, 70 << 32 | 70);
        for index in 0..70usize {
            let wing = 0x1000 + index * 0x800;
            put(0x400 + index * 8, wing as u64);
            put(wing, cair_wing::VTABLE_RVA.0 as u64);
            put(wing + 0xc0, if index % 3 == 0 { REGION as u64 } else { 0 });
            put(wing + 0x90 + crate::air_mission::WING, wing as u64);
            put(wing + 0xa0, 4 | index as u64);
            put(wing + 0x30, 4);
            put(wing + cair_wing::PLANE_COUNT, 10 + index as u64);
        }
        put(0x400 + 69 * 8, 0x1000 + 5 * 0x800);
        bytes
    }

    fn run(bytes: &[u8], rows: &mut Vec<Row>) -> (Result<(), &'static str>, usize) {
        let mut sink = Sink {
            rows: crate::walk::Buffer::Growing(rows),
            required: 0,
            raid: 7,
            ordinal: 0,
        };
        let pools = Array {
            data: 0x100,
            count: 1,
        };
        let memory = SliceMemory { base: 0, bytes };
        let result = interceptors(&memory, 0, pools, (REGION, 61), &mut sink);
        (result, sink.required)
    }

    #[test]
    fn only_target_region_wings_are_read_in_occurrence_order() {
        let bytes = world();
        let mut rows = vec![Row::default(); 200];
        let (result, required) = run(&bytes, &mut rows);
        assert_eq!(result, Ok(()));
        assert_eq!(required, 23 * 4);
        let planes: Vec<_> = rows[..required]
            .iter()
            .filter(|row| row.kind == INTERCEPTION_PLANES)
            .map(|row| (row.key, row.raw))
            .collect();
        assert_eq!(planes[..3], [(0, 10), (1, 13), (2, 16)]);
        assert!(
            rows[..required]
                .iter()
                .filter(|row| row.kind == INTERCEPTION_REGION)
                .all(|row| row.raw == 61)
        );
    }

    #[test]
    fn a_repeated_wing_counts_twice_and_a_bad_wing_fails_the_raid() {
        let mut bytes = world();
        bytes[0x400 + 69 * 8..0x400 + 70 * 8]
            .copy_from_slice(&(0x1000u64 + 3 * 0x800).to_le_bytes());
        let mut rows = vec![Row::default(); 200];
        let (result, required) = run(&bytes, &mut rows);
        assert_eq!((result, required), (Ok(()), 24 * 4));
        assert_eq!(rows[required - 1].raw, 13);
        let wing = 0x1000 + 40 * 0x800;
        bytes[wing] ^= 1;
        assert_eq!(
            run(&bytes, &mut rows).0,
            Err("raid tooltip context vtable mismatch")
        );
    }
}
