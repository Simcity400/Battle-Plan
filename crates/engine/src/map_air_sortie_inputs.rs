//! Stored operands of flying-aircraft map admission and count.
//!
//! The map recomputes each wing's displayed count from sortie efficiency.
//! These readers copy the stored operands; the viewer applies the calculation.
//! Stored mission efficiency and count can be stale because their writer runs
//! only when period, capacity and combat gates pass (see `AirMissionInputRow`).

use crate::{
    Memory, air_mission, cair_base, ccountry, ccountry_air_container, cgame_state,
    cpdx_array::Array,
    csub_unit_definition, cweather_manager,
    memory::{RecordField, RecordLayout, RecordSources},
    modifier_accumulator, modifier_definition, read, read_slice, sweather_per_region,
    walk::Buffer,
};

/// Modifier key read from every sortie-efficiency source.
pub const SORTIE_MODIFIER_KEY: u32 = 0x11;
/// Mission signed fixed5 region coverage.
const REGION_COVERAGE: usize = 0x50;
/// Mission signed fixed5 reassignment penalty. Its complement scales sortie
/// efficiency; changing region resets it from the matching content define.
const REASSIGNMENT_PENALTY: usize = 0xc0;
/// Signed fixed5 CAS diversion ratio, independent of the count cache.
const CAS_RATIO: usize = 0xd0;
/// Sixteen-byte province ID and distance entries.
const REACHED_ENTRY: usize = 0x10;
/// Console efficiency overrides: a PdxArray of 16-byte canonical-slot i32,
/// padding and fixed5 i64 entries, unique by slot. A matching effective-country
/// slot supplies the result before any other term or clamp.
const EFFICIENCY_OVERRIDES_RVA: usize = 0x0331_7208;
const OVERRIDE_ENTRY: usize = 0x10;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Mission {
    pub coverage_raw: i64,
    pub reassignment_penalty_raw: i64,
    /// +0x70; zero when no target was chosen.
    pub last_target_province: i32,
    /// The reached vector (+0x38, count +0x44), acquisition context only.
    pub reached: Array,
    /// The mission's CStrategicRegion*, acquisition context only.
    pub region_pointer: usize,
    /// Mission operand weighting the CAS diversion draw.
    pub cas_ratio_raw: Option<i64>,
}

fn field<T: crate::memory::Raw>(
    memory: &impl Memory,
    object: usize,
    offset: usize,
) -> Result<T, &'static str> {
    object
        .checked_add(offset)
        .and_then(|at| read(memory, at))
        .ok_or("map sortie operand unreadable")
}

fn pointer(memory: &impl Memory, object: usize, offset: usize) -> Result<usize, &'static str> {
    usize::try_from(field::<u64>(memory, object, offset)?)
        .map_err(|_| "map sortie pointer overflow")
}

fn vtable(base: usize, rva: usize) -> Result<u64, &'static str> {
    base.checked_add(rva)
        .map(|value| value as u64)
        .ok_or("map sortie module address overflow")
}

/// Missions read per `copy_records` batch.
pub const MISSION_BATCH: usize = 64;
const MISSION_ROW: usize = 56;
const MISSION_FIELDS: [RecordField; 6] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: air_mission::REGION,
        destination_offset: 8,
        len: 8,
    },
    RecordField {
        source_offset: air_mission::REACHED_PROVINCES_DATA,
        destination_offset: 16,
        len: 16,
    },
    RecordField {
        source_offset: REGION_COVERAGE,
        destination_offset: 32,
        len: 8,
    },
    RecordField {
        source_offset: REASSIGNMENT_PENALTY,
        destination_offset: 40,
        len: 8,
    },
    RecordField {
        source_offset: air_mission::LAST_TARGET_PROVINCE,
        destination_offset: 48,
        len: 4,
    },
];
const MISSION_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&MISSION_FIELDS, MISSION_ROW).expect("valid mission layout");

fn word<const N: usize>(row: &[u8], at: usize) -> [u8; N] {
    row[at..at + N].try_into().expect("fixed mission field")
}

/// Up to [`MISSION_BATCH`] registered missions' operands in one gathered copy;
/// the reached IDs and pool definitions are separate reads.
pub fn read_missions(
    memory: &impl Memory,
    base: usize,
    missions: &[usize],
    output: &mut [Mission],
) -> Result<(), &'static str> {
    if missions.len() > MISSION_BATCH || output.len() != missions.len() {
        return Err("air mission batch size invalid");
    }
    let mut rows = [0_u8; MISSION_BATCH * MISSION_ROW];
    let rows = &mut rows[..missions.len() * MISSION_ROW];
    if memory.copy_records(RecordSources::Pointers(missions), &MISSION_LAYOUT, rows)
        != missions.len() * MISSION_FIELDS.len()
    {
        return Err("air mission operands unreadable");
    }
    let identity = vtable(base, air_mission::VTABLE_RVA)?;
    for ((mission, row), &pointer) in output
        .iter_mut()
        .zip(rows.chunks_exact(MISSION_ROW))
        .zip(missions)
    {
        if u64::from_le_bytes(word(row, 0)) != identity {
            return Err("air mission identity mismatch");
        }
        *mission = Mission {
            cas_ratio_raw: pointer
                .checked_add(CAS_RATIO)
                .and_then(|at| read(memory, at)),
            coverage_raw: i64::from_le_bytes(word(row, 32)),
            reassignment_penalty_raw: i64::from_le_bytes(word(row, 40)),
            last_target_province: i32::from_le_bytes(word(row, 48)),
            reached: Array::from_header(word(row, 16), REACHED_ENTRY)
                .ok_or("air mission reached vector invalid")?,
            region_pointer: usize::try_from(u64::from_le_bytes(word(row, 8)))
                .map_err(|_| "air mission region pointer overflow")?,
        };
    }
    Ok(())
}

/// A wing pool's definition AIR_FLAGS; zero for the null sub-unit singleton.
pub fn definition_air_flags(
    memory: &impl Memory,
    base: usize,
    pool: usize,
) -> Result<u64, &'static str> {
    let definition = pointer(memory, pool, crate::cair_wing_pool::DEFINITION)?;
    match field::<u64>(memory, definition, 0)? {
        real if real == vtable(base, csub_unit_definition::VTABLE_RVA)? => {
            field(memory, definition, csub_unit_definition::AIR_FLAGS)
        }
        null if null == vtable(base, csub_unit_definition::NULL_VTABLE_RVA)? => Ok(0),
        _ => Err("air wing pool definition identity mismatch"),
    }
}

/// The reached province IDs in native order; `output` holds exactly its count.
pub fn read_reached(
    memory: &impl Memory,
    reached: Array,
    output: &mut [u32],
) -> Result<(), &'static str> {
    if output.len() != reached.count as usize {
        return Err("air mission reached buffer size differs from its count");
    }
    let mut entries = [0_u8; 64 * REACHED_ENTRY];
    for (batch, ids) in output.chunks_mut(64).enumerate() {
        let at = reached
            .element((batch * 64) as u32, REACHED_ENTRY)
            .ok_or("air mission reached address overflow")?;
        let bytes = &mut entries[..ids.len() * REACHED_ENTRY];
        if !read_slice(memory, at, bytes) {
            return Err("air mission reached provinces unreadable");
        }
        for (id, entry) in ids.iter_mut().zip(bytes.chunks_exact(REACHED_ENTRY)) {
            *id = u32::from_le_bytes(entry[..4].try_into().unwrap());
        }
    }
    Ok(())
}

/// A base's per-country capacity, supply ratio, fuel demand and allocation.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Container {
    pub capacity_raw: i64,
    pub supply_raw: i64,
    pub fuel_demand_raw: i64,
    pub fuel_allocated_raw: i64,
}

/// A base's per-country container vector (`CAirBase+0x30`), read once per base.
pub fn country_containers(memory: &impl Memory, air_base: usize) -> Result<Array, &'static str> {
    air_base
        .checked_add(cair_base::COUNTRY_CONTAINERS)
        .and_then(|at| Array::read(memory, at, 8))
        .ok_or("air base country containers invalid")
}

/// `containers` is [`country_containers`]; `slot` a canonical country slot.
/// `None` for a null container slot, which the law treats as all zero.
pub fn read_container<M: Memory>(
    memory: &M,
    base: usize,
    containers: Array,
    slot: u32,
    countries: &ccountry::Reader<'_, M>,
) -> Result<Option<Container>, &'static str> {
    let entry = containers
        .element(slot, 8)
        .ok_or("air base country container outside its array")?;
    let container = pointer(memory, entry, 0)?;
    if container == 0 {
        return Ok(None);
    }
    let at = |offset: usize| container.checked_add(offset).unwrap_or(0);
    let [
        mut identity,
        mut supply,
        mut capacity,
        mut demand,
        mut allocated,
    ] = [[0_u8; 8]; 5];
    let mut owner = [0_u8; 4];
    if memory.copy_scattered(
        [
            container,
            at(ccountry_air_container::SUPPLY_RATIO_RAW),
            at(ccountry_air_container::COUNTRY),
            at(ccountry_air_container::SORTIE_CAPACITY_FACTOR_RAW),
            at(ccountry_air_container::FUEL_DEMAND_RAW),
            at(ccountry_air_container::FUEL_ALLOCATED_RAW),
        ],
        &mut [
            &mut identity,
            &mut supply,
            &mut owner,
            &mut capacity,
            &mut demand,
            &mut allocated,
        ],
    ) != 6
    {
        return Err("air base country container unreadable");
    }
    let owner = u32::from_le_bytes(owner);
    if u64::from_le_bytes(identity) != vtable(base, ccountry_air_container::VTABLE_RVA)?
        || owner == 0
        || countries.link(owner)? != Some(slot)
    {
        return Err("air base country container identity mismatch");
    }
    Ok(Some(Container {
        capacity_raw: i64::from_le_bytes(capacity),
        supply_raw: i64::from_le_bytes(supply),
        fuel_demand_raw: i64::from_le_bytes(demand),
        fuel_allocated_raw: i64::from_le_bytes(allocated),
    }))
}

/// The weather manager's per-region records, read once per hour.
pub fn weather_regions(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<Array, &'static str> {
    let manager = pointer(memory, game_state, cgame_state::WEATHER_MANAGER)?;
    if manager == 0
        || field::<u64>(memory, manager, 0)? != vtable(base, cweather_manager::VTABLE_RVA)?
    {
        return Err("weather manager identity mismatch");
    }
    manager
        .checked_add(cweather_manager::REGIONS)
        .and_then(|at| Array::read(memory, at, sweather_per_region::STRIDE))
        .ok_or("weather regions invalid")
}

/// The weather region's key-0x11 accumulator entry.
/// `regions` is [`weather_regions`].
pub fn weather_modifier(
    memory: &impl Memory,
    base: usize,
    regions: Array,
    region: u32,
) -> Result<modifier_accumulator::Value, &'static str> {
    let record = regions
        .element(region, sweather_per_region::STRIDE)
        .ok_or("weather region outside its array")?;
    let mut identity = [0_u8; 12];
    if !read_slice(memory, record, &mut identity)
        || u64::from_le_bytes(identity[..8].try_into().unwrap())
            != vtable(base, sweather_per_region::VTABLE_RVA)?
        || u32::from_le_bytes(identity[8..].try_into().unwrap()) != region
    {
        return Err("weather region identity mismatch");
    }
    modifier_accumulator::lookup_at(
        memory,
        record + sweather_per_region::MODIFIERS,
        SORTIE_MODIFIER_KEY,
    )
}

/// The effective country's own key-0x11 entry (CCountry+0x5B8); `country`
/// is an authenticated CCountry.
pub fn country_modifier(
    memory: &impl Memory,
    country: usize,
) -> Result<modifier_accumulator::Value, &'static str> {
    modifier_accumulator::lookup_at(
        memory,
        country
            .checked_add(ccountry::MODIFIER_ACCUMULATOR)
            .ok_or("country modifier address overflow")?,
        SORTIE_MODIFIER_KEY,
    )
}

/// Key 0x11's runtime definition: clamp flag, feature gate and whether the
/// gate callback supplies the feature-flags mask.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ModifierMetadata {
    pub flags: u32,
    pub feature_selector: u32,
    /// 0: null callback; 1: the feature-flags getter.
    pub callback_kind: u8,
}

pub fn read_modifier_metadata(
    memory: &impl Memory,
    base: usize,
) -> Result<ModifierMetadata, &'static str> {
    let definitions = pointer(memory, base, modifier_definition::TABLE_RVA)?;
    if definitions == 0 {
        return Err("modifier definitions absent");
    }
    let metadata = definitions + SORTIE_MODIFIER_KEY as usize * modifier_definition::STRIDE;
    let feature_selector = field(memory, metadata, modifier_definition::FEATURE_SELECTOR)?;
    let callback_kind = if feature_selector == 0 {
        0
    } else {
        match field::<u64>(memory, base, modifier_definition::ELIGIBILITY_CALLBACK_RVA)? {
            0 => 0,
            getter
                if getter == vtable(base, modifier_definition::ELIGIBILITY_FLAGS_GETTER_RVA)? =>
            {
                1
            }
            _ => return Err("modifier eligibility callback unsupported"),
        }
    };
    Ok(ModifierMetadata {
        flags: field(memory, metadata, modifier_definition::FLAGS)?,
        feature_selector,
        callback_kind,
    })
}

/// The console efficiency overrides in native order, as `(slot, raw)`.
pub fn efficiency_overrides(
    memory: &impl Memory,
    base: usize,
    mut visit: impl FnMut(u32, i64),
) -> Result<(), &'static str> {
    efficiency_overrides_fallible(memory, base, |slot, raw| {
        visit(slot, raw);
        Ok(())
    })
}

pub fn efficiency_overrides_fallible(
    memory: &impl Memory,
    base: usize,
    mut visit: impl FnMut(u32, i64) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let overrides = base
        .checked_add(EFFICIENCY_OVERRIDES_RVA)
        .and_then(|at| Array::read(memory, at, OVERRIDE_ENTRY))
        .ok_or("sortie efficiency overrides invalid")?;
    let mut entries = [0_u8; 64 * OVERRIDE_ENTRY];
    for first in (0..overrides.count).step_by(64) {
        let count = (overrides.count - first).min(64);
        let bytes = &mut entries[..count as usize * OVERRIDE_ENTRY];
        let at = overrides
            .element(first, OVERRIDE_ENTRY)
            .ok_or("sortie efficiency override address overflow")?;
        if !read_slice(memory, at, bytes) {
            return Err("sortie efficiency overrides unreadable");
        }
        for entry in bytes.chunks_exact(OVERRIDE_ENTRY) {
            visit(
                u32::from_le_bytes(entry[..4].try_into().unwrap()),
                i64::from_le_bytes(entry[8..].try_into().unwrap()),
            )?;
        }
    }
    Ok(())
}

/// Raw references in a country's outgoing volunteer-host vector. Only these
/// references admit a base controller. Returns the vector count and fills
/// `output` only when it fits.
pub fn volunteer_hosts(
    memory: &impl Memory,
    country: usize,
    output: &mut [u32],
) -> Result<usize, &'static str> {
    volunteer_hosts_buffer(memory, country, Buffer::Fixed(output))
}

pub fn volunteer_hosts_growing(
    memory: &impl Memory,
    country: usize,
    output: &mut Vec<u32>,
) -> Result<usize, &'static str> {
    volunteer_hosts_buffer(memory, country, Buffer::Growing(output))
}

fn volunteer_hosts_buffer(
    memory: &impl Memory,
    country: usize,
    mut output: Buffer<'_, u32>,
) -> Result<usize, &'static str> {
    let hosts = country
        .checked_add(ccountry::VOLUNTEER_HOSTS)
        .and_then(|at| Array::read(memory, at, 4))
        .ok_or("country volunteer hosts invalid")?;
    // Admission reaches the complete outgoing volunteer-host vector.
    output.ensure(hosts.count as usize)?;
    if let Some(output) = output.get_mut(..hosts.count as usize) {
        crate::cpdx_array::copy_u32s(memory, hosts, output, "country volunteer hosts unreadable")?;
    }
    Ok(hosts.count as usize)
}

/// Off-thread storage demand; no mission value survives this hint.
pub fn storage_demand(
    memory: &impl Memory,
    base: usize,
    state: usize,
) -> Result<[usize; 4], &'static str> {
    let countries = ccountry::Reader::new(memory, base, state)?;
    let mut hosts = 0;
    for slot in 1..countries.arrays()[0].count {
        if let Some(country) = countries.at(slot)? {
            hosts = hosts.max(volunteer_hosts(memory, country.pointer, &mut [])?);
        }
    }
    let mut missions = 0usize;
    let mut reached = 0usize;
    crate::cstrategic_air::visit(memory, base, state, |root| {
        let active = root
            .address
            .checked_add(crate::cstrategic_air::ALL_ACTIVE_MISSIONS)
            .and_then(|at| Array::read(memory, at, 8))
            .ok_or("sortie sizing active missions invalid")?;
        missions = missions
            .checked_add(active.count as usize)
            .ok_or("sortie sizing mission count overflow")?;
        crate::cpdx_array::pointers(
            memory,
            active,
            "sortie sizing mission pointer unreadable",
            |_, mission| {
                if mission == 0
                    || read::<u64>(memory, mission) != Some(vtable(base, air_mission::VTABLE_RVA)?)
                {
                    return Err("sortie sizing mission identity mismatch");
                }
                let header = mission
                    .checked_add(air_mission::REACHED_PROVINCES_DATA)
                    .and_then(|at| Array::read(memory, at, REACHED_ENTRY))
                    .ok_or("sortie sizing reached vector invalid")?;
                reached = reached
                    .checked_add(header.count as usize)
                    .ok_or("sortie sizing reached count overflow")?;
                Ok(())
            },
        )
    })?;
    let overrides = base
        .checked_add(EFFICIENCY_OVERRIDES_RVA)
        .and_then(|at| Array::read(memory, at, OVERRIDE_ENTRY))
        .ok_or("sortie sizing efficiency overrides invalid")?;
    Ok([missions, reached, hosts, overrides.count as usize])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::SliceMemory;

    const MISSION: usize = 0x1000;
    const POOL: usize = 0x2000;
    const DEFINITION: usize = 0x3000;
    const REACHED: usize = 0x4000;

    fn put(bytes: &mut [u8], at: usize, value: &[u8]) {
        bytes[at..at + value.len()].copy_from_slice(value);
    }

    fn mission_fixture() -> Vec<u8> {
        let mut bytes = vec![0; 0x5000];
        put(
            &mut bytes,
            MISSION,
            &(air_mission::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            MISSION + air_mission::REACHED_PROVINCES_DATA,
            &(REACHED as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            MISSION + air_mission::REACHED_PROVINCES_CAPACITY,
            &3_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            MISSION + air_mission::REACHED_PROVINCES_COUNT,
            &2_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            MISSION + REGION_COVERAGE,
            &(-7_i64).to_le_bytes(),
        );
        put(
            &mut bytes,
            MISSION + REASSIGNMENT_PENALTY,
            &40_000_i64.to_le_bytes(),
        );
        put(
            &mut bytes,
            MISSION + air_mission::LAST_TARGET_PROVINCE,
            &11_i32.to_le_bytes(),
        );
        put(&mut bytes, REACHED, &5_u32.to_le_bytes());
        put(&mut bytes, REACHED + REACHED_ENTRY, &3_u32.to_le_bytes());
        put(
            &mut bytes,
            POOL + crate::cair_wing_pool::DEFINITION,
            &(DEFINITION as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            DEFINITION,
            &(csub_unit_definition::VTABLE_RVA as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            DEFINITION + csub_unit_definition::AIR_FLAGS,
            &0x1c_0001_0000_u64.to_le_bytes(),
        );
        bytes
    }

    #[test]
    fn mission_operands_and_reached_ids_keep_native_values_and_order() {
        let bytes = mission_fixture();
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut missions = [Mission::default()];
        read_missions(&memory, 0, &[MISSION], &mut missions).unwrap();
        let mission = missions[0];
        let mut reached = [0; 2];
        read_reached(&memory, mission.reached, &mut reached).unwrap();
        assert_eq!(reached, [5, 3]);
        assert_eq!(
            (mission.coverage_raw, mission.reassignment_penalty_raw),
            (-7, 40_000)
        );
        assert_eq!(
            (mission.last_target_province, mission.reached.count),
            (11, 2)
        );
        assert_eq!(definition_air_flags(&memory, 0, POOL), Ok(0x1c_0001_0000));
        assert!(read_reached(&memory, mission.reached, &mut [0; 1]).is_err());
        assert!(read_missions(&memory, 0, &[MISSION, POOL], &mut missions).is_err());
        assert_eq!(
            read_missions(&memory, 0, &[MISSION, POOL], &mut [Mission::default(); 2]),
            Err("air mission identity mismatch")
        );
    }

    #[test]
    fn cas_ratio_keeps_observed_zero_signed_values_and_an_optional_read_fault() {
        struct Fault<'a>(&'a [u8]);
        impl Memory for Fault<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                if address <= MISSION + CAS_RATIO
                    && MISSION + CAS_RATIO < address.saturating_add(destination.len())
                {
                    return false;
                }
                SliceMemory {
                    base: 0,
                    bytes: self.0,
                }
                .copy(address, destination)
            }
        }
        let mut bytes = mission_fixture();
        let mut rows = [Mission::default()];
        for value in [0_i64, i64::MIN, i64::MAX, -7] {
            put(&mut bytes, MISSION + CAS_RATIO, &value.to_le_bytes());
            read_missions(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                0,
                &[MISSION],
                &mut rows,
            )
            .unwrap();
            assert_eq!(rows[0].cas_ratio_raw, Some(value));
        }
        read_missions(&Fault(&bytes), 0, &[MISSION], &mut rows).unwrap();
        assert_eq!(rows[0].cas_ratio_raw, None);
        assert_eq!(rows[0].coverage_raw, -7);
        assert_eq!(rows[0].reached.count, 2);
    }

    #[test]
    fn original_host_header_grows_5000_and_later_mod_occurrences_once() {
        use std::cell::Cell;
        struct Traced<'a> {
            bytes: &'a [u8],
            header: Cell<usize>,
        }
        impl Memory for Traced<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address == 0x1000 + ccountry::VOLUNTEER_HOSTS {
                    self.header.set(self.header.get() + 1);
                }
                SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(address, output)
            }
        }
        let mut output = Vec::new();
        for count in [5_000usize, 12_345] {
            let data = 0x3000;
            let mut bytes = vec![0; data + count * 4];
            let header = 0x1000 + ccountry::VOLUNTEER_HOSTS;
            bytes[header..header + 8].copy_from_slice(&(data as u64).to_le_bytes());
            bytes[header + 8..header + 12].copy_from_slice(&(count as u32).to_le_bytes());
            bytes[header + 12..header + 16].copy_from_slice(&(count as u32).to_le_bytes());
            for index in 0..count {
                bytes[data + index * 4..data + index * 4 + 4]
                    .copy_from_slice(&(index as u32).to_le_bytes());
            }
            let memory = Traced {
                bytes: &bytes,
                header: Cell::new(0),
            };
            assert_eq!(
                volunteer_hosts_growing(&memory, 0x1000, &mut output),
                Ok(count)
            );
            assert_eq!(memory.header.get(), 1);
            assert_eq!(output[count - 1], count as u32 - 1);
            assert_eq!(volunteer_hosts(&memory, 0x1000, &mut []), Ok(count));
        }
    }

    #[test]
    fn efficiency_overrides_keep_native_order_and_values() {
        let mut bytes = vec![0; EFFICIENCY_OVERRIDES_RVA + 0x100];
        let entries = EFFICIENCY_OVERRIDES_RVA + 0x20;
        put(
            &mut bytes,
            EFFICIENCY_OVERRIDES_RVA,
            &(entries as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            EFFICIENCY_OVERRIDES_RVA + 8,
            &2_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            EFFICIENCY_OVERRIDES_RVA + 12,
            &2_u32.to_le_bytes(),
        );
        put(&mut bytes, entries, &9_u32.to_le_bytes());
        put(&mut bytes, entries + 8, &0_i64.to_le_bytes());
        put(&mut bytes, entries + 0x10, &4_u32.to_le_bytes());
        put(&mut bytes, entries + 0x18, &(-5_i64).to_le_bytes());
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let mut seen = Vec::new();
        efficiency_overrides(&memory, 0, |slot, raw| seen.push((slot, raw))).unwrap();
        assert_eq!(seen, [(9, 0), (4, -5)]);
        put(
            &mut bytes,
            EFFICIENCY_OVERRIDES_RVA + 12,
            &3_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            EFFICIENCY_OVERRIDES_RVA + 8,
            &3_u32.to_le_bytes(),
        );
        let tail = (bytes.len() - 0x10) as u64;
        put(&mut bytes, EFFICIENCY_OVERRIDES_RVA, &tail.to_le_bytes());
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert!(efficiency_overrides(&memory, 0, |_, _| {}).is_err());
    }

    #[test]
    fn the_null_definition_has_no_flags_and_a_foreign_one_is_an_error() {
        let mut bytes = mission_fixture();
        put(
            &mut bytes,
            DEFINITION,
            &(csub_unit_definition::NULL_VTABLE_RVA as u64).to_le_bytes(),
        );
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert_eq!(definition_air_flags(&memory, 0, POOL), Ok(0));
        put(&mut bytes, DEFINITION, &1_u64.to_le_bytes());
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        assert!(definition_air_flags(&memory, 0, POOL).is_err());
    }
}
