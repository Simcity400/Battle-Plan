//! Stored operands of flying-aircraft map admission and count.
//!
//! The map recomputes each wing's displayed count from sortie efficiency.
//! These readers copy the stored operands; the viewer applies the calculation.
//! Stored mission efficiency and count can be stale because their writer runs
//! only when period, capacity and combat gates pass (see `AirMissionInputRow`).

use engine_native::cair_wing;
use engine_sites::Rva;

#[path = "map_air_sortie_inputs/patrol.rs"]
pub mod patrol;

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
/// Retained base-to-region trace count tested by the regional interception alert.
const INTERCEPTION_TRACE_COUNT: usize = 0x64;
/// Sixteen-byte province ID and distance entries.
const REACHED_ENTRY: usize = 0x10;
/// Console efficiency overrides: a PdxArray of 16-byte canonical-slot i32,
/// padding and fixed5 i64 entries, unique by slot. A matching effective-country
/// slot supplies the result before any other term or clamp.
const EFFICIENCY_OVERRIDES_RVA: Rva = Rva(0x033d_da48);
/// Console command's sole detection-vector writer and verified entry fingerprint.
pub const DETECTION_OVERRIDE_WRITER_RVA: Rva = Rva(0x0027_ffc0);
pub const DETECTION_OVERRIDE_WRITER_PREFIX_HASH: u64 = 0x18aa_1a4c_14d9_dba7;
/// The console's country-keyed fixed5 detection vector.
const DETECTION_OVERRIDES_RVA: Rva = Rva(0x033d_da60);
const OVERRIDE_ENTRY: usize = 0x10;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Mission {
    pub coverage_raw: i64,
    pub reassignment_penalty_raw: i64,
    /// The chosen target; zero when no target was chosen.
    pub last_target_province: i32,
    /// The reached vector (with its count), acquisition context only.
    pub reached: Array,
    /// The mission's CStrategicRegion*, acquisition context only.
    pub region_pointer: usize,
    /// Mission operand weighting the CAS diversion draw.
    pub cas_ratio_raw: Option<i64>,
    /// Exact trace-count predicate, independent of current aircraft and sortie efficiency.
    pub interception_alert_gate: Option<bool>,
    /// Current simulation execution, independent of the enabled mission mask.
    pub current_type: Option<u32>,
    /// Reached only for a nonzero current type; stale while execution is zero.
    pub effective_aircraft: Option<i32>,
    pub normalization: Option<Normalization>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Normalization {
    pub night_stat: Option<i64>,
    pub modifiers: [Option<modifier_accumulator::RequestedOperand>; 2],
    /// Independent cached aircraft detection leaves for regional Navy spotting.
    pub detection: [Option<i64>; 2],
    pub patrol: patrol::Inputs,
}

const SURFACE_DETECTION_STAT: usize = 0x13;
const SUBMARINE_DETECTION_STAT: usize = 0x14;

fn detection_stats(memory: &impl Memory, record: usize) -> [Option<i64>; 2] {
    let Some(first) = record.checked_add(SURFACE_DETECTION_STAT * 8) else {
        return [None; 2];
    };
    let mut bytes = [0; 16];
    if memory.copy(first, &mut bytes) {
        return [
            Some(i64::from_le_bytes(bytes[..8].try_into().unwrap())),
            Some(i64::from_le_bytes(bytes[8..].try_into().unwrap())),
        ];
    }
    // A fault in either word cannot hide the independent readable neighbour.
    [SURFACE_DETECTION_STAT, SUBMARINE_DETECTION_STAT]
        .map(|stat| record.checked_add(stat * 8).and_then(|at| read(memory, at)))
}

fn normalization(
    memory: &impl Memory,
    base: usize,
    mission: usize,
    mask: u32,
    count: i32,
    spotting: bool,
) -> Normalization {
    let Some(wing) = mission.checked_sub(cair_wing::MISSION) else {
        return Normalization::default();
    };
    let record = cair_wing::mission_stat_index(memory, wing, mask)
        .ok()
        .and_then(|index| cair_wing::mission_stat_record(memory, wing, index).ok());
    let night_stat = record
        .and_then(|record| record.checked_add(cair_wing::NIGHT_OPERATIONS_STAT * 8))
        .and_then(|at| read(memory, at));
    let detection = if spotting {
        record
            .map(|record| detection_stats(memory, record))
            .unwrap_or([None; 2])
    } else {
        [None; 2]
    };
    let modifiers = [0x105_u32, 0x106].map(|key| {
        wing.checked_add(cair_wing::MODIFIERS)
            .and_then(|at| modifier_accumulator::requested_operand_at(memory, base, at, key).ok())
    });
    Normalization {
        night_stat,
        modifiers,
        detection,
        patrol: if spotting {
            patrol::read(memory, wing, mission, record, mask, count)
        } else {
            Default::default()
        },
    }
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

fn vtable(base: usize, rva: Rva) -> Result<u64, &'static str> {
    engine_sites::at(base, rva)
        .map(|value| value as u64)
        .ok_or("map sortie module address overflow")
}

fn global<T: crate::memory::Raw>(
    memory: &impl Memory,
    base: usize,
    key: Rva,
) -> Result<T, &'static str> {
    engine_sites::at(base, key)
        .and_then(|at| read(memory, at))
        .ok_or("map sortie operand unreadable")
}

/// Missions read per `copy_records` batch.
pub const MISSION_BATCH: usize = 64;
const MISSION_ROW: usize = 72;
/// The six operand fields every mission must read.
const OPERAND_FIELDS: [RecordField; 6] = [
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
/// Required operands, then the independently optional CAS ratio and interception threshold.
const MISSION_FIELDS: [RecordField; 9] = {
    let mut fields = [RecordField {
        source_offset: CAS_RATIO,
        destination_offset: 56,
        len: 8,
    }; 9];
    fields[7] = RecordField {
        source_offset: INTERCEPTION_TRACE_COUNT,
        destination_offset: 52,
        len: 4,
    };
    fields[8] = RecordField {
        source_offset: air_mission::CURRENT_TYPE,
        destination_offset: 64,
        len: 4,
    };
    let mut index = 0;
    while index < OPERAND_FIELDS.len() {
        fields[index] = OPERAND_FIELDS[index];
        index += 1;
    }
    fields
};
/// Required operands only; optional operands are then read independently.
const MISSION_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&OPERAND_FIELDS, MISSION_ROW).expect("valid mission layout");

fn word<const N: usize>(row: &[u8], at: usize) -> [u8; N] {
    row[at..at + N].try_into().expect("fixed mission field")
}

/// Up to [`MISSION_BATCH`] registered missions' operands and CAS ratios in
/// one gathered copy; the reached IDs and pool definitions are separate
/// reads. When that copy stops, the operands are copied without the ratios,
/// which must all read, and each ratio is read alone, an unreadable one
/// unknown: what the batch and the ratios' own reads give. That fallback
/// is counted (`batch_fallback`); true when the one copy read
/// them all.
pub fn read_missions(
    memory: &impl Memory,
    base: usize,
    missions: &[usize],
    output: &mut [Mission],
) -> Result<bool, &'static str> {
    read_missions_with_spotting(
        memory,
        base,
        missions,
        None,
        &mut Default::default(),
        output,
    )
}

/// Reuse the wing worker's authenticated current-execution leaves, including
/// their independent failures. Neither a mask nor its count is read again.
#[cfg(test)]
pub fn read_missions_with_execution(
    memory: &impl Memory,
    base: usize,
    missions: &[usize],
    execution: &[cair_wing::fuel_inputs::Inputs],
    output: &mut [Mission],
) -> Result<bool, &'static str> {
    read_missions_with_spotting(
        memory,
        base,
        missions,
        Some(execution),
        &mut Default::default(),
        output,
    )
}

/// The worker shares this stack cache across every original mission batch.
pub fn read_missions_with_spotting(
    memory: &impl Memory,
    base: usize,
    missions: &[usize],
    execution: Option<&[cair_wing::fuel_inputs::Inputs]>,
    spotting: &mut patrol::SpottingFactors,
    output: &mut [Mission],
) -> Result<bool, &'static str> {
    if execution.is_some_and(|execution| execution.len() != missions.len()) {
        return Err("air mission execution batch size invalid");
    }
    read_missions_inner(memory, base, missions, execution, spotting, output)
}

fn read_missions_inner(
    memory: &impl Memory,
    base: usize,
    missions: &[usize],
    execution: Option<&[cair_wing::fuel_inputs::Inputs]>,
    spotting: &mut patrol::SpottingFactors,
    output: &mut [Mission],
) -> Result<bool, &'static str> {
    if missions.len() > MISSION_BATCH || output.len() != missions.len() {
        return Err("air mission batch size invalid");
    }
    let mut rows = [0_u8; MISSION_BATCH * MISSION_ROW];
    let rows = &mut rows[..missions.len() * MISSION_ROW];
    let fields = if execution.is_some() {
        &MISSION_FIELDS[..MISSION_FIELDS.len() - 1]
    } else {
        &MISSION_FIELDS[..]
    };
    let layout = RecordLayout::new(fields, MISSION_ROW).expect("valid mission execution layout");
    let gathered = memory.copy_records(RecordSources::Pointers(missions), &layout, rows)
        == missions.len() * fields.len();
    if !gathered {
        // Counted as it starts, whatever the reads after it return.
        crate::batch_fallback::record(crate::batch_fallback::Batch::MissionInputs);
        if memory.copy_records(RecordSources::Pointers(missions), &MISSION_LAYOUT, rows)
            != missions.len() * MISSION_LAYOUT.field_count()
        {
            return Err("air mission operands unreadable");
        }
    }
    let identity = vtable(base, air_mission::VTABLE_RVA)?;
    for (index, ((mission, row), &pointer)) in output
        .iter_mut()
        .zip(rows.chunks_exact(MISSION_ROW))
        .zip(missions)
        .enumerate()
    {
        if u64::from_le_bytes(word(row, 0)) != identity {
            return Err("air mission identity mismatch");
        }
        *mission = Mission {
            current_type: if let Some(execution) = execution {
                execution[index].current_type.map(|value| value as u32)
            } else if gathered {
                Some(u32::from_le_bytes(word(row, 64)))
            } else {
                pointer
                    .checked_add(air_mission::CURRENT_TYPE)
                    .and_then(|at| read(memory, at))
            },
            effective_aircraft: execution.and_then(|rows| rows[index].effective_aircraft),
            normalization: None,
            interception_alert_gate: if gathered {
                Some(i32::from_le_bytes(word(row, 52)) > 1)
            } else {
                pointer
                    .checked_add(INTERCEPTION_TRACE_COUNT)
                    .and_then(|at| read::<i32>(memory, at))
                    .map(|count| count > 1)
            },
            cas_ratio_raw: if gathered {
                Some(i64::from_le_bytes(word(row, 56)))
            } else {
                pointer
                    .checked_add(CAS_RATIO)
                    .and_then(|at| read(memory, at))
            },
            coverage_raw: i64::from_le_bytes(word(row, 32)),
            reassignment_penalty_raw: i64::from_le_bytes(word(row, 40)),
            last_target_province: i32::from_le_bytes(word(row, 48)),
            reached: Array::from_header(word(row, 16), REACHED_ENTRY)
                .ok_or("air mission reached vector invalid")?,
            region_pointer: usize::try_from(u64::from_le_bytes(word(row, 8)))
                .map_err(|_| "air mission region pointer overflow")?,
        };
    }
    if execution.is_none() {
        read_effective_aircraft(memory, missions, output);
    }
    for (&pointer, mission) in missions.iter().zip(output.iter_mut()) {
        if let (Some(mask), Some(count)) = (mission.current_type, mission.effective_aircraft)
            && mask != 0
            && count != 0
        {
            mission.normalization = Some(normalization(
                memory,
                base,
                pointer,
                mask,
                count,
                spotting.admits(memory, base, mask).unwrap_or(false),
            ));
        }
    }
    Ok(gathered)
}

/// Only the batch's executing missions demand this retained count. All
/// storage is bounded by the existing gather size, never the wing census.
fn read_effective_aircraft(memory: &impl Memory, missions: &[usize], output: &mut [Mission]) {
    const FIELDS: [RecordField; 1] = [RecordField {
        source_offset: air_mission::EFFECTIVE_AIRCRAFTS,
        destination_offset: 0,
        len: 4,
    }];
    const LAYOUT: RecordLayout<'static> =
        RecordLayout::new(&FIELDS, 4).expect("valid executing aircraft layout");
    let mut pointers = [0; MISSION_BATCH];
    let mut indices = [0; MISSION_BATCH];
    let mut count = 0;
    for (index, (&pointer, row)) in missions.iter().zip(output.iter()).enumerate() {
        if row.current_type.is_some_and(|mask| mask != 0) {
            pointers[count] = pointer;
            indices[count] = index;
            count += 1;
        }
    }
    if count == 0 {
        return;
    }
    let mut words = [0_u8; MISSION_BATCH * 4];
    let copied = memory.copy_records(
        RecordSources::Pointers(&pointers[..count]),
        &LAYOUT,
        &mut words[..count * 4],
    );
    for at in 0..count {
        output[indices[at]].effective_aircraft = if at < copied {
            Some(i32::from_le_bytes(word(&words, at * 4)))
        } else if at == copied {
            // The failed gather leaf stays unknown; do not retry it.
            None
        } else {
            pointers[at]
                .checked_add(air_mission::EFFECTIVE_AIRCRAFTS)
                .and_then(|address| read(memory, address))
        };
    }
}

/// Pools whose definitions [`definition_air_flags_gathered`] reads at once.
pub const DEFINITION_BATCH: usize = 64;
const POOL_DEFINITION: [RecordField; 1] = [RecordField {
    source_offset: crate::cair_wing_pool::DEFINITION,
    destination_offset: 0,
    len: 8,
}];
const POOL_DEFINITION_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&POOL_DEFINITION, 8).expect("valid pool definition layout");
const DEFINITION_FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: 0,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: csub_unit_definition::AIR_FLAGS,
        destination_offset: 8,
        len: 8,
    },
];
const DEFINITION_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&DEFINITION_FIELDS, 16).expect("valid definition layout");

/// [`definition_air_flags`] of up to [`DEFINITION_BATCH`] `pools`: their
/// definition pointers in one copy, then each definition's identity and
/// flags in one (the flags read ahead of the identity check). Returns how
/// many leading pools it read, their flags in `output`; it stops at the
/// first pool a copy did not complete for or whose identity refuses,
/// counting that fallback (`batch_fallback`), and the caller
/// reads that pool and the rest with [`definition_air_flags`].
pub fn definition_air_flags_gathered(
    memory: &impl Memory,
    base: usize,
    pools: &[usize],
    output: &mut [u64],
) -> usize {
    let count = pools.len().min(output.len()).min(DEFINITION_BATCH);
    let read = gather_definitions(memory, base, &pools[..count], output);
    if read < count {
        crate::batch_fallback::record(crate::batch_fallback::Batch::PoolDefinitions);
    }
    read
}

fn gather_definitions(
    memory: &impl Memory,
    base: usize,
    pools: &[usize],
    output: &mut [u64],
) -> usize {
    let count = pools.len();
    let mut pointers = [0_u8; DEFINITION_BATCH * 8];
    let read = memory.copy_records(
        RecordSources::Pointers(&pools[..count]),
        &POOL_DEFINITION_LAYOUT,
        &mut pointers[..count * 8],
    );
    let mut definitions = [0_usize; DEFINITION_BATCH];
    for (definition, pointer) in definitions[..read].iter_mut().zip(pointers.chunks_exact(8)) {
        *definition = u64::from_le_bytes(pointer.try_into().unwrap()) as usize;
    }
    let mut records = [0_u8; DEFINITION_BATCH * 16];
    let read = memory.copy_records(
        RecordSources::Pointers(&definitions[..read]),
        &DEFINITION_LAYOUT,
        &mut records[..read * 16],
    ) / DEFINITION_FIELDS.len();
    let (Ok(real), Ok(null)) = (
        vtable(base, csub_unit_definition::VTABLE_RVA),
        vtable(base, csub_unit_definition::NULL_VTABLE_RVA),
    ) else {
        return 0;
    };
    for (index, record) in records[..read * 16].chunks_exact(16).enumerate() {
        output[index] = match u64::from_le_bytes(record[..8].try_into().unwrap()) {
            identity if identity == real => u64::from_le_bytes(record[8..].try_into().unwrap()),
            identity if identity == null => 0,
            _ => return index,
        };
    }
    read
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

/// A base's per-country container vector (`cair_base::COUNTRY_CONTAINERS`), read once per base.
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
    weather_regions_from_manager(memory, weather_manager(memory, base, game_state)?)
}

/// Authenticated acquisition context, loaned to consumers in the same worker.
pub fn weather_manager(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
) -> Result<usize, &'static str> {
    let manager = pointer(memory, game_state, cgame_state::WEATHER_MANAGER)?;
    if manager == 0
        || field::<u64>(memory, manager, 0)? != vtable(base, cweather_manager::VTABLE_RVA)?
    {
        return Err("weather manager identity mismatch");
    }
    Ok(manager)
}

/// `manager` is the original successful [`weather_manager`] observation.
pub fn weather_regions_from_manager(
    memory: &impl Memory,
    manager: usize,
) -> Result<Array, &'static str> {
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

/// The effective country's own key-0x11 entry (`ccountry::MODIFIER_ACCUMULATOR`); `country` is an
/// authenticated CCountry.
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
    let definitions = usize::try_from(global::<u64>(memory, base, modifier_definition::TABLE_RVA)?)
        .map_err(|_| "map sortie pointer overflow")?;
    if definitions == 0 {
        return Err("modifier definitions absent");
    }
    let metadata = definitions + SORTIE_MODIFIER_KEY as usize * modifier_definition::STRIDE;
    let feature_selector = field(memory, metadata, modifier_definition::FEATURE_SELECTOR)?;
    let callback_kind = if feature_selector == 0 {
        0
    } else {
        match global::<u64>(memory, base, modifier_definition::ELIGIBILITY_CALLBACK_RVA)? {
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
#[cfg(test)]
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
    let overrides = engine_sites::at(base, EFFICIENCY_OVERRIDES_RVA)
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

/// First/change-only caller-owned acquisition; never call this as a periodic poll.
/// The native header supplies allocation demand before any entry is followed.
pub fn detection_overrides(
    memory: &impl Memory,
    base: usize,
    output: &mut Vec<(u32, i64)>,
) -> Result<(), &'static str> {
    output.clear();
    let array = engine_sites::at(base, DETECTION_OVERRIDES_RVA)
        .and_then(|at| Array::read(memory, at, OVERRIDE_ENTRY))
        .ok_or("Air detection override vector header unreadable")?;
    engine_native::allocation::reserve_exact(output, array.count as usize)
        .map_err(|_| "Air detection override allocation failed")?;
    let mut bytes = [0_u8; 64 * OVERRIDE_ENTRY];
    for first in (0..array.count).step_by(64) {
        let count = (array.count - first).min(64);
        let source = array
            .element(first, OVERRIDE_ENTRY)
            .ok_or("Air detection override vector extent invalid")?;
        let bytes = &mut bytes[..count as usize * OVERRIDE_ENTRY];
        if !memory.copy(source, bytes) {
            return Err("Air detection override vector entry unreadable");
        }
        for row in bytes.chunks_exact(OVERRIDE_ENTRY) {
            output.push((
                u32::from_le_bytes(row[..4].try_into().unwrap()),
                i64::from_le_bytes(row[8..16].try_into().unwrap()),
            ));
        }
    }
    output.sort_unstable_by_key(|row| row.0);
    if output.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err("Air detection override vector repeats a country");
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
    let overrides = engine_sites::at(base, EFFICIENCY_OVERRIDES_RVA)
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
            &(air_mission::VTABLE_RVA.0 as u64).to_le_bytes(),
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
            &(csub_unit_definition::VTABLE_RVA.0 as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            DEFINITION + csub_unit_definition::AIR_FLAGS,
            &0x1c_0001_0000_u64.to_le_bytes(),
        );
        bytes
    }

    #[test]
    fn normalization_reads_only_executing_nonzero_counts_and_keeps_independent_faults() {
        struct Traced<'a> {
            bytes: &'a [u8],
            fault: Option<usize>,
            reads: std::cell::RefCell<Vec<(usize, usize)>>,
        }
        impl Memory for Traced<'_> {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                self.reads.borrow_mut().push((at, out.len()));
                const FACTORS: usize = 0x8000_0000;
                if at == patrol::SPOTTING_FACTORS_RVA.0 {
                    let mut header = [0; 16];
                    header[..8].copy_from_slice(&(FACTORS as u64).to_le_bytes());
                    header[8..12].copy_from_slice(&32_u32.to_le_bytes());
                    header[12..].copy_from_slice(&32_u32.to_le_bytes());
                    out.copy_from_slice(&header);
                    return true;
                }
                if (FACTORS..FACTORS + 32 * 8).contains(&at) {
                    out.copy_from_slice(&100_000_i64.to_le_bytes());
                    return true;
                }
                if self
                    .fault
                    .is_some_and(|fault| at <= fault && fault < at + out.len())
                {
                    return false;
                }
                SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(at, out)
            }
        }
        let wing = MISSION - cair_wing::MISSION;
        let selector = wing + cair_wing::MISSION_STATS + cair_wing::MISSION_STAT_INDEXES + 3;
        let statistic = wing + cair_wing::DEFAULT_STATS + cair_wing::NIGHT_OPERATIONS_STAT * 8;
        let surface = wing + cair_wing::DEFAULT_STATS + SURFACE_DETECTION_STAT * 8;
        let submarine = wing + cair_wing::DEFAULT_STATS + SUBMARINE_DETECTION_STAT * 8;
        let accumulator = wing + cair_wing::MODIFIERS;
        let mut bytes = mission_fixture();
        put(&mut bytes, selector, &[255]);
        put(&mut bytes, statistic, &(-123_456_i64).to_le_bytes());
        put(&mut bytes, surface, &i64::MIN.to_le_bytes());
        put(&mut bytes, submarine, &i64::MAX.to_le_bytes());
        let mut output = [Mission::default()];
        for (mask, count, fault) in [
            (0_u32, 99_i32, None),
            (8, 0, None),
            (8, 17, None),
            (8, 17, Some(statistic)),
            (8, 17, Some(accumulator)),
            (8, 17, Some(surface)),
            (8, 17, Some(submarine)),
        ] {
            put(
                &mut bytes,
                MISSION + air_mission::CURRENT_TYPE,
                &mask.to_le_bytes(),
            );
            put(
                &mut bytes,
                MISSION + air_mission::EFFECTIVE_AIRCRAFTS,
                &count.to_le_bytes(),
            );
            let memory = Traced {
                bytes: &bytes,
                fault,
                reads: Default::default(),
            };
            read_missions(&memory, 0, &[MISSION], &mut output).unwrap();
            if mask == 0 || count == 0 {
                assert!(output[0].normalization.is_none());
                assert!(memory.reads.borrow().iter().all(|&(at, _)| at != selector
                    && at != statistic
                    && at != accumulator
                    && at != surface
                    && at != submarine));
            } else {
                let packet = output[0].normalization.unwrap();
                assert_eq!(
                    packet.night_stat,
                    (fault != Some(statistic)).then_some(-123_456)
                );
                assert_eq!(
                    packet.detection,
                    [
                        (fault != Some(surface)).then_some(i64::MIN),
                        (fault != Some(submarine)).then_some(i64::MAX),
                    ]
                );
                assert_eq!(
                    memory
                        .reads
                        .borrow()
                        .iter()
                        .filter(|&&(at, _)| at == selector)
                        .count(),
                    1
                );
                assert_eq!(
                    packet
                        .modifiers
                        .map(|entry| entry.map(|entry| entry.present)),
                    if fault == Some(accumulator) {
                        [None; 2]
                    } else {
                        [Some(false); 2]
                    }
                );
            }
            assert_eq!(output[0].coverage_raw, -7);
        }
    }

    #[test]
    fn zero_or_unreadable_spotting_factor_skips_new_leaves_but_keeps_air_normalization() {
        struct Gate<'a> {
            bytes: &'a [u8],
            factor: Option<i64>,
            new_reads: std::cell::Cell<usize>,
        }
        impl Memory for Gate<'_> {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                const FACTORS: usize = 0x8000_0000;
                if at == patrol::SPOTTING_FACTORS_RVA.0 {
                    let mut header = [0; 16];
                    header[..8].copy_from_slice(&(FACTORS as u64).to_le_bytes());
                    header[8..12].copy_from_slice(&8_u32.to_le_bytes());
                    header[12..].copy_from_slice(&8_u32.to_le_bytes());
                    out.copy_from_slice(&header);
                    return true;
                }
                if at == FACTORS + 3 * 8 {
                    let Some(value) = self.factor else {
                        return false;
                    };
                    out.copy_from_slice(&value.to_le_bytes());
                    return true;
                }
                let wing = MISSION - cair_wing::MISSION;
                let record = wing + cair_wing::DEFAULT_STATS;
                if [
                    MISSION + 0xa8,
                    record + 0x13 * 8,
                    record + 0x14 * 8,
                    record + 0x2f * 8,
                    record + 0x30 * 8,
                    wing + 0x580,
                ]
                .contains(&at)
                {
                    self.new_reads.set(self.new_reads.get() + 1);
                }
                SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(at, out)
            }
        }
        let mut bytes = mission_fixture();
        let wing = MISSION - cair_wing::MISSION;
        put(
            &mut bytes,
            MISSION + air_mission::CURRENT_TYPE,
            &8_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            MISSION + air_mission::EFFECTIVE_AIRCRAFTS,
            &17_i32.to_le_bytes(),
        );
        put(
            &mut bytes,
            wing + cair_wing::MISSION_STATS + cair_wing::MISSION_STAT_INDEXES + 3,
            &[255],
        );
        put(
            &mut bytes,
            wing + cair_wing::DEFAULT_STATS + cair_wing::NIGHT_OPERATIONS_STAT * 8,
            &123_i64.to_le_bytes(),
        );
        for factor in [Some(0), None, Some(100_000)] {
            let memory = Gate {
                bytes: &bytes,
                factor,
                new_reads: Default::default(),
            };
            let mut output = [Mission::default()];
            read_missions(&memory, 0, &[MISSION], &mut output).unwrap();
            let normalization = output[0].normalization.unwrap();
            assert_eq!(normalization.night_stat, Some(123));
            assert_eq!(
                normalization
                    .modifiers
                    .map(|value| value.map(|value| value.present)),
                [Some(false); 2]
            );
            if factor == Some(100_000) {
                assert!(memory.new_reads.get() > 0);
                assert_eq!(normalization.detection, [Some(0); 2]);
                assert_eq!(normalization.patrol.disruption, Some(0));
            } else {
                assert_eq!(memory.new_reads.get(), 0);
                assert_eq!(normalization.detection, [None; 2]);
                assert_eq!(normalization.patrol, Default::default());
            }
        }
    }

    #[test]
    fn patrol_detection_does_not_read_stats_for_invalid_current_masks() {
        struct Guard<'a> {
            bytes: &'a [u8],
            attempts: std::cell::Cell<usize>,
        }
        impl Memory for Guard<'_> {
            fn copy(&self, at: usize, out: &mut [u8]) -> bool {
                let wing = MISSION - cair_wing::MISSION;
                let start = wing + cair_wing::DEFAULT_STATS + SURFACE_DETECTION_STAT * 8;
                if at < start + 16 && start < at.saturating_add(out.len()) {
                    self.attempts.set(self.attempts.get() + 1);
                    return false;
                }
                SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(at, out)
            }
        }
        let mut bytes = mission_fixture();
        for mask in [3_u32, 0x8000_0000] {
            put(
                &mut bytes,
                MISSION + air_mission::CURRENT_TYPE,
                &mask.to_le_bytes(),
            );
            put(
                &mut bytes,
                MISSION + air_mission::EFFECTIVE_AIRCRAFTS,
                &7_i32.to_le_bytes(),
            );
            let memory = Guard {
                bytes: &bytes,
                attempts: Default::default(),
            };
            let mut output = [Mission::default()];
            read_missions(&memory, 0, &[MISSION], &mut output).unwrap();
            assert_eq!(output[0].normalization.unwrap().detection, [None; 2]);
            assert_eq!(memory.attempts.get(), 0);
        }
    }

    #[test]
    fn executing_count_is_demanded_only_by_current_mask_and_faults_are_independent() {
        use std::cell::Cell;
        struct Traced<'a> {
            bytes: &'a [u8],
            deny: Option<usize>,
            count_reads: Cell<usize>,
        }
        impl Memory for Traced<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let end = address.saturating_add(output.len());
                if address <= MISSION + air_mission::EFFECTIVE_AIRCRAFTS
                    && MISSION + air_mission::EFFECTIVE_AIRCRAFTS < end
                {
                    self.count_reads.set(self.count_reads.get() + 1);
                }
                if self.deny.is_some_and(|at| address <= at && at < end) {
                    return false;
                }
                SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(address, output)
            }
        }
        let mut bytes = mission_fixture();
        put(
            &mut bytes,
            MISSION + air_mission::EFFECTIVE_AIRCRAFTS,
            &(-17_i32).to_le_bytes(),
        );
        let mut rows = [Mission::default()];
        for (mask, deny, expected_mask, expected_count, reads) in [
            (0_u32, None, Some(0), None, 0),
            (8, None, Some(8), Some(-17), 1),
            (
                8,
                Some(MISSION + air_mission::EFFECTIVE_AIRCRAFTS),
                Some(8),
                None,
                1,
            ),
            (8, Some(MISSION + air_mission::CURRENT_TYPE), None, None, 0),
        ] {
            put(
                &mut bytes,
                MISSION + air_mission::CURRENT_TYPE,
                &mask.to_le_bytes(),
            );
            let memory = Traced {
                bytes: &bytes,
                deny,
                count_reads: Cell::new(0),
            };
            read_missions(&memory, 0, &[MISSION], &mut rows).unwrap();
            assert_eq!(
                (rows[0].current_type, rows[0].effective_aircraft),
                (expected_mask, expected_count)
            );
            assert_eq!(memory.count_reads.get(), reads);
            assert_eq!(rows[0].coverage_raw, -7);
        }
    }

    #[test]
    fn same_pass_execution_reuse_never_rereads_masks_counts_or_heals_failed_leaves() {
        struct NoExecution<'a> {
            bytes: &'a [u8],
            attempts: std::cell::Cell<usize>,
        }
        impl Memory for NoExecution<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                let end = address.saturating_add(output.len());
                if [air_mission::CURRENT_TYPE, air_mission::EFFECTIVE_AIRCRAFTS]
                    .into_iter()
                    .any(|offset| address <= MISSION + offset && MISSION + offset < end)
                {
                    self.attempts.set(self.attempts.get() + 1);
                    return false;
                }
                SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(address, output)
            }
        }
        let bytes = mission_fixture();
        let memory = NoExecution {
            bytes: &bytes,
            attempts: Default::default(),
        };
        let mut output = [Mission::default()];
        for (mask, aircraft) in [(Some(0), None), (Some(8), Some(0)), (None, None)] {
            let execution = [cair_wing::fuel_inputs::Inputs {
                current_type: mask,
                effective_aircraft: aircraft,
                ..Default::default()
            }];
            read_missions_with_execution(&memory, 0, &[MISSION], &execution, &mut output).unwrap();
            assert_eq!(output[0].current_type, mask.map(|value| value as u32));
            assert_eq!(output[0].effective_aircraft, aircraft);
            assert_eq!(output[0].coverage_raw, -7);
        }
        assert_eq!(memory.attempts.get(), 0);
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
    fn interception_trace_gate_uses_the_retained_signed_count_and_refuses_only_its_field() {
        struct Fault<'a>(&'a [u8]);
        impl Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address <= MISSION + INTERCEPTION_TRACE_COUNT
                    && MISSION + INTERCEPTION_TRACE_COUNT < address.saturating_add(output.len())
                {
                    return false;
                }
                SliceMemory {
                    base: 0,
                    bytes: self.0,
                }
                .copy(address, output)
            }
        }
        let mut bytes = mission_fixture();
        let mut rows = [Mission::default()];
        for (count, expected) in [
            (i32::MIN, false),
            (0, false),
            (1, false),
            (2, true),
            (i32::MAX, true),
        ] {
            put(
                &mut bytes,
                MISSION + INTERCEPTION_TRACE_COUNT,
                &count.to_le_bytes(),
            );
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
            assert_eq!(rows[0].interception_alert_gate, Some(expected));
        }
        read_missions(&Fault(&bytes), 0, &[MISSION], &mut rows).unwrap();
        assert_eq!(rows[0].interception_alert_gate, None);
        assert_eq!(rows[0].coverage_raw, -7);
        assert!(rows[0].cas_ratio_raw.is_some());
        assert_eq!(MISSION_ROW, 72);
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
    fn detection_override_vector_grows_from_its_header_and_preserves_zero_and_signed_words() {
        let mut bytes = vec![0_u8; 32 + 5_000 * OVERRIDE_ENTRY];
        put(
            &mut bytes,
            0,
            &((DETECTION_OVERRIDES_RVA.0 + 32) as u64).to_le_bytes(),
        );
        put(&mut bytes, 8, &5_000_u32.to_le_bytes());
        put(&mut bytes, 12, &5_000_u32.to_le_bytes());
        for at in 0..5_000_usize {
            put(
                &mut bytes,
                32 + at * OVERRIDE_ENTRY,
                &(4_999 - at as u32).to_le_bytes(),
            );
            let value = if at == 0 { i64::MIN } else { -(at as i64) };
            put(
                &mut bytes,
                32 + at * OVERRIDE_ENTRY + 8,
                &value.to_le_bytes(),
            );
        }
        let mut rows = Vec::new();
        detection_overrides(
            &SliceMemory {
                base: DETECTION_OVERRIDES_RVA.0,
                bytes: &bytes,
            },
            0,
            &mut rows,
        )
        .unwrap();
        assert_eq!(rows.len(), 5_000);
        assert_eq!(rows[0], (0, -4_999));
        assert_eq!(rows[4_999], (4_999, i64::MIN));
        let capacity = rows.capacity();
        put(&mut bytes, 12, &1_u32.to_le_bytes());
        put(&mut bytes, 32, &7_u32.to_le_bytes());
        put(&mut bytes, 40, &0_i64.to_le_bytes());
        detection_overrides(
            &SliceMemory {
                base: DETECTION_OVERRIDES_RVA.0,
                bytes: &bytes,
            },
            0,
            &mut rows,
        )
        .unwrap();
        assert_eq!(rows, [(7, 0)]);
        assert_eq!(rows.capacity(), capacity);
        put(&mut bytes, 12, &2_u32.to_le_bytes());
        put(&mut bytes, 48, &7_u32.to_le_bytes());
        assert!(
            detection_overrides(
                &SliceMemory {
                    base: DETECTION_OVERRIDES_RVA.0,
                    bytes: &bytes
                },
                0,
                &mut rows
            )
            .is_err()
        );
    }

    #[test]
    fn efficiency_overrides_keep_native_order_and_values() {
        let mut bytes = vec![0; EFFICIENCY_OVERRIDES_RVA.0 + 0x100];
        let entries = EFFICIENCY_OVERRIDES_RVA.0 + 0x20;
        put(
            &mut bytes,
            EFFICIENCY_OVERRIDES_RVA.0,
            &(entries as u64).to_le_bytes(),
        );
        put(
            &mut bytes,
            EFFICIENCY_OVERRIDES_RVA.0 + 8,
            &2_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            EFFICIENCY_OVERRIDES_RVA.0 + 12,
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
            EFFICIENCY_OVERRIDES_RVA.0 + 12,
            &3_u32.to_le_bytes(),
        );
        put(
            &mut bytes,
            EFFICIENCY_OVERRIDES_RVA.0 + 8,
            &3_u32.to_le_bytes(),
        );
        let tail = (bytes.len() - 0x10) as u64;
        put(&mut bytes, EFFICIENCY_OVERRIDES_RVA.0, &tail.to_le_bytes());
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
            &(csub_unit_definition::NULL_VTABLE_RVA.0 as u64).to_le_bytes(),
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

    /// The operands and CAS ratios of a batch take one gathered copy.
    #[test]
    fn mission_operands_and_cas_ratios_take_one_copy() {
        let mut bytes = mission_fixture();
        put(&mut bytes, MISSION + CAS_RATIO, &(-3_i64).to_le_bytes());
        let memory = crate::memory::CountingMemory::new(&bytes);
        let mut rows = [Mission::default(); 2];
        assert_eq!(
            read_missions(&memory, 0, &[MISSION, MISSION], &mut rows),
            Ok(true)
        );
        assert_eq!(memory.calls.get(), 1);
        assert_eq!(rows.map(|row| row.cas_ratio_raw), [Some(-3); 2]);
    }

    /// `pools` pools over a quarter as many definitions, the null singleton
    /// every fifth; definition `d` holds flags `d << 8`.
    fn definition_world(pools: usize) -> (Vec<u8>, Vec<usize>) {
        const POOLS: usize = 0x1000;
        /// Room for a definition through its flags.
        const DEFINITION: usize = (csub_unit_definition::AIR_FLAGS + 8).next_multiple_of(0x40);
        let definitions = pools / 4 + 1;
        let first = POOLS + pools * 0x40;
        let mut bytes = vec![0; first + definitions * DEFINITION];
        for definition in 0..definitions {
            let at = first + definition * DEFINITION;
            let vtable = if definition % 5 == 0 {
                csub_unit_definition::NULL_VTABLE_RVA
            } else {
                csub_unit_definition::VTABLE_RVA
            };
            put(&mut bytes, at, &(vtable.0 as u64).to_le_bytes());
            put(
                &mut bytes,
                at + csub_unit_definition::AIR_FLAGS,
                &((definition as u64) << 8).to_le_bytes(),
            );
        }
        let pools = (0..pools)
            .map(|pool| {
                let at = POOLS + pool * 0x40;
                let definition = first + (pool % definitions) * DEFINITION;
                put(
                    &mut bytes,
                    at + crate::cair_wing_pool::DEFINITION,
                    &(definition as u64).to_le_bytes(),
                );
                at
            })
            .collect();
        (bytes, pools)
    }

    /// The flags read before the first refusal, and that refusal.
    type Flags = (Vec<u64>, Result<(), &'static str>);

    fn flags_serial(memory: &impl Memory, pools: &[usize]) -> Flags {
        let mut flags = Vec::new();
        for &pool in pools {
            match definition_air_flags(memory, 0, pool) {
                Ok(value) => flags.push(value),
                Err(reason) => return (flags, Err(reason)),
            }
        }
        (flags, Ok(()))
    }

    /// The gathered reads with their per-pool fallbacks, as the collector
    /// makes them; with the fallbacks, each also counted for status.
    fn flags_gathered(memory: &impl Memory, pools: &[usize]) -> (Flags, usize) {
        use crate::batch_fallback::{Batch, count};
        let counted = count(Batch::PoolDefinitions);
        let mut flags = Vec::new();
        let mut fallbacks = 0;
        let mut output = [0; DEFINITION_BATCH];
        let result = (|| {
            for chunk in pools.chunks(DEFINITION_BATCH) {
                let read = definition_air_flags_gathered(memory, 0, chunk, &mut output);
                fallbacks += usize::from(read < chunk.len());
                flags.extend_from_slice(&output[..read]);
                for &pool in &chunk[read..] {
                    flags.push(definition_air_flags(memory, 0, pool)?);
                }
            }
            Ok(())
        })();
        assert_eq!(count(Batch::PoolDefinitions) - counted, fallbacks as u64);
        ((flags, result), fallbacks)
    }

    /// Healthy, 150 pools' gathered flags are the per-pool reads', in two
    /// copies a 64 pools rather than three reads a pool.
    #[test]
    fn gathered_definitions_equal_the_per_pool_reads() {
        let (bytes, pools) = definition_world(150);
        let memory = SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let expected = flags_serial(&memory, &pools);
        assert_eq!((expected.0.len(), expected.1), (pools.len(), Ok(())));
        assert!(expected.0.contains(&0) && expected.0.iter().any(|&flags| flags != 0));
        assert_eq!(flags_gathered(&memory, &pools), (expected, 0));
        let counted = |gather: bool| {
            let memory = crate::memory::CountingMemory::new(&bytes);
            let read = if gather {
                flags_gathered(&memory, &pools).0
            } else {
                flags_serial(&memory, &pools)
            };
            assert_eq!(read.1, Ok(()));
            memory.calls.get()
        };
        let (serial, gathered) = (counted(false), counted(true));
        assert!(serial >= gathered + 2 * pools.len(), "{serial} {gathered}");
    }

    /// A persistent fault gives the per-pool result, one that clears after
    /// its first touch the healthy or the per-pool reads' own; a null
    /// definition's flags, read ahead, refuse nothing.
    #[test]
    fn gathered_definition_faults_refuse_no_more_than_the_per_pool_reads() {
        struct Faulted<'a> {
            bytes: &'a [u8],
            fault: usize,
            once: bool,
            met: std::cell::Cell<bool>,
        }
        impl Memory for Faulted<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                let touches = address <= self.fault && self.fault < address + destination.len();
                if touches && !(self.once && self.met.replace(true)) {
                    return false;
                }
                SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(address, destination)
            }
        }
        let (bytes, pools) = definition_world(150);
        let healthy = flags_serial(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            &pools,
        );
        let definition = |pool: usize| {
            let at = pools[pool] + crate::cair_wing_pool::DEFINITION;
            u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap()) as usize
        };
        let faulted = |fault, once| Faulted {
            bytes: &bytes,
            fault,
            once,
            met: std::cell::Cell::new(false),
        };
        for pool in [0_usize, 1, 63, 64, 70, 149] {
            for fault in [
                pools[pool] + crate::cair_wing_pool::DEFINITION,
                definition(pool),
                definition(pool) + csub_unit_definition::AIR_FLAGS,
            ] {
                let reference = flags_serial(&faulted(fault, false), &pools);
                let null = pool % 38 % 5 == 0;
                let flags = fault == definition(pool) + csub_unit_definition::AIR_FLAGS;
                assert_eq!(reference == healthy, null && flags, "{fault:x}");
                assert_eq!(
                    flags_gathered(&faulted(fault, false), &pools).0,
                    reference,
                    "{fault:x}"
                );
                let once = flags_gathered(&faulted(fault, true), &pools).0;
                let reference = flags_serial(&faulted(fault, true), &pools);
                assert!(once == healthy || once == reference, "{fault:x}: {once:?}");
            }
        }
    }

    /// `read_missions` as it stood before the ratios joined the batch
    /// (eafc953b4f): the six operand fields in one copy, each ratio alone.
    fn read_missions_parent(
        memory: &impl Memory,
        base: usize,
        missions: &[usize],
        output: &mut [Mission],
    ) -> Result<(), &'static str> {
        const ROW: usize = 56;
        const LAYOUT: RecordLayout<'static> =
            RecordLayout::new(&OPERAND_FIELDS, ROW).expect("valid mission layout");
        if missions.len() > MISSION_BATCH || output.len() != missions.len() {
            return Err("air mission batch size invalid");
        }
        let mut rows = [0_u8; MISSION_BATCH * ROW];
        let rows = &mut rows[..missions.len() * ROW];
        if memory.copy_records(RecordSources::Pointers(missions), &LAYOUT, rows)
            != missions.len() * OPERAND_FIELDS.len()
        {
            return Err("air mission operands unreadable");
        }
        let identity = vtable(base, air_mission::VTABLE_RVA)?;
        for ((mission, row), &pointer) in
            output.iter_mut().zip(rows.chunks_exact(ROW)).zip(missions)
        {
            if u64::from_le_bytes(word(row, 0)) != identity {
                return Err("air mission identity mismatch");
            }
            *mission = Mission {
                current_type: pointer
                    .checked_add(air_mission::CURRENT_TYPE)
                    .and_then(|at| read(memory, at)),
                effective_aircraft: None,
                normalization: None,
                interception_alert_gate: pointer
                    .checked_add(INTERCEPTION_TRACE_COUNT)
                    .and_then(|at| read::<i32>(memory, at))
                    .map(|count| count > 1),
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
        read_effective_aircraft(memory, missions, output);
        let mut spotting = patrol::SpottingFactors::default();
        for (&pointer, mission) in missions.iter().zip(output.iter_mut()) {
            if let (Some(mask), Some(count)) = (mission.current_type, mission.effective_aircraft)
                && mask != 0
                && count != 0
            {
                mission.normalization = Some(normalization(
                    memory,
                    base,
                    pointer,
                    mask,
                    count,
                    spotting.admits(memory, base, mask).unwrap_or(false),
                ));
            }
        }
        Ok(())
    }

    const MISSIONS: usize = 0x10_000;
    const MISSION_SIZE: usize = 0x100;

    /// 70 missions, two batches, with distinct operands and ratios.
    fn missions_world() -> (Vec<u8>, Vec<usize>) {
        let mut bytes = vec![0; MISSIONS + 70 * MISSION_SIZE + 0x1000];
        let reached = MISSIONS + 70 * MISSION_SIZE;
        let missions = (0..70)
            .map(|index| {
                let at = MISSIONS + index * MISSION_SIZE;
                put(
                    &mut bytes,
                    at,
                    &(air_mission::VTABLE_RVA.0 as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    at + air_mission::REGION,
                    &(0x9000 + index as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    at + air_mission::REACHED_PROVINCES_DATA,
                    &(reached as u64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    at + air_mission::REACHED_PROVINCES_CAPACITY,
                    &3_u32.to_le_bytes(),
                );
                put(
                    &mut bytes,
                    at + air_mission::REACHED_PROVINCES_COUNT,
                    &(index as u32 % 3).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    at + REGION_COVERAGE,
                    &(index as i64).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    at + REASSIGNMENT_PENALTY,
                    &(-(index as i64)).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    at + air_mission::LAST_TARGET_PROVINCE,
                    &(index as i32 + 1).to_le_bytes(),
                );
                put(
                    &mut bytes,
                    at + CAS_RATIO,
                    &(index as i64 * 3).to_le_bytes(),
                );
                at
            })
            .collect();
        (bytes, missions)
    }

    /// Each batch's result and every output row, partly written ones too.
    type Batches = Vec<(Result<(), &'static str>, Vec<Mission>)>;

    fn batches(memory: &impl Memory, missions: &[usize], parent: bool) -> (Batches, u64) {
        use crate::batch_fallback::{Batch, count};
        let counted = count(Batch::MissionInputs);
        let batches = missions
            .chunks(MISSION_BATCH)
            .map(|chunk| {
                let mut output = vec![Mission::default(); chunk.len()];
                let result = if parent {
                    read_missions_parent(memory, 0, chunk, &mut output)
                } else {
                    read_missions(memory, 0, chunk, &mut output).map(drop)
                };
                (result, output)
            })
            .collect();
        (batches, count(Batch::MissionInputs) - counted)
    }

    /// Against the parent reader: a persistent fault gives its result, each
    /// output row included, and counts one fallback per batch it stops even
    /// when the retry refuses; a fault that clears after its first touch
    /// gives the healthy rows or the parent's; a ratio faulted once
    /// withholds nothing; malformed and foreign missions at the batch
    /// boundary refuse as the parent does.
    #[test]
    fn mission_batches_refuse_no_more_than_the_parent_reader() {
        struct Faulted<'a> {
            bytes: &'a [u8],
            faults: Vec<usize>,
            once: bool,
            met: std::cell::RefCell<Vec<bool>>,
        }
        impl Memory for Faulted<'_> {
            fn copy(&self, address: usize, destination: &mut [u8]) -> bool {
                let mut met = self.met.borrow_mut();
                for (&fault, met) in self.faults.iter().zip(met.iter_mut()) {
                    if address <= fault && fault < address + destination.len() && !*met {
                        *met = self.once;
                        return false;
                    }
                }
                SliceMemory {
                    base: 0,
                    bytes: self.bytes,
                }
                .copy(address, destination)
            }
        }
        let (mut bytes, missions) = missions_world();
        let healthy = batches(
            &SliceMemory {
                base: 0,
                bytes: &bytes,
            },
            &missions,
            true,
        )
        .0;
        assert!(healthy.iter().all(|(result, _)| result.is_ok()));
        assert_eq!(
            batches(
                &SliceMemory {
                    base: 0,
                    bytes: &bytes,
                },
                &missions,
                false
            ),
            (healthy.clone(), 0)
        );
        let mission = |index: usize| missions[index];
        let mut cases: Vec<Vec<usize>> = Vec::new();
        for index in [0, 63, 64, 69] {
            for offset in [
                0,
                air_mission::REGION,
                air_mission::REACHED_PROVINCES_DATA,
                air_mission::REACHED_PROVINCES_COUNT,
                REGION_COVERAGE,
                REASSIGNMENT_PENALTY,
                air_mission::LAST_TARGET_PROVINCE,
                CAS_RATIO,
            ] {
                cases.push(vec![mission(index) + offset]);
            }
        }
        // Several failures: two ratios and a later required operand.
        cases.push(vec![
            mission(3) + CAS_RATIO,
            mission(40) + CAS_RATIO,
            mission(50) + REGION_COVERAGE,
        ]);
        cases.push(vec![mission(3) + CAS_RATIO, mission(65) + CAS_RATIO]);
        for faults in &cases {
            let run = |parent, once| {
                let memory = Faulted {
                    bytes: &bytes,
                    faults: faults.clone(),
                    once,
                    met: std::cell::RefCell::new(vec![false; faults.len()]),
                };
                batches(&memory, &missions, parent)
            };
            let (reference, _) = run(true, false);
            assert_ne!(reference, healthy, "{faults:x?}");
            let stopped = faults
                .iter()
                .map(|&fault| (fault - MISSIONS) / MISSION_SIZE / MISSION_BATCH)
                .collect::<std::collections::BTreeSet<_>>()
                .len() as u64;
            assert_eq!(run(false, false), (reference, stopped), "{faults:x?}");
            let (once, _) = run(false, true);
            assert!(
                once == healthy || once == run(true, true).0,
                "{faults:x?}: {once:?}"
            );
            if faults
                .iter()
                .all(|fault| (fault - MISSIONS) % MISSION_SIZE == CAS_RATIO)
            {
                assert_eq!(once, healthy, "a ratio faulted once withholds nothing");
            }
        }
        // A malformed reached header and a foreign mission at the boundary.
        for (index, offset, value) in [
            (63, air_mission::REACHED_PROVINCES_COUNT, 9_u64),
            (64, 0, 0),
            (63, 0, 7),
        ] {
            let at = mission(index) + offset;
            let saved = bytes[at..at + 8].to_vec();
            put(
                &mut bytes,
                at,
                &value.to_le_bytes()[..if offset == 0 { 8 } else { 4 }],
            );
            let memory = SliceMemory {
                base: 0,
                bytes: &bytes,
            };
            let reference = batches(&memory, &missions, true).0;
            assert_ne!(reference, healthy);
            assert_eq!(batches(&memory, &missions, false), (reference, 0));
            put(&mut bytes, at, &saved);
        }
    }
}
