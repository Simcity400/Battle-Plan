use engine_native::ctask_force::mission_layout::{
    CHARACTER_UNIT_LEADER,
    CHARACTER_UNIT_LEADER_REFERENCE,
    COMMANDER_CHARACTER,
    FUEL,
    HEADQUARTERS_COMMANDER,
    LEADER_MODIFIERS,
    LOCAL_MODIFIERS,
    REPLENISHMENT,
    SOURCE_MODIFIERS,
    STAT_RANGE,
    UNDERWAY,
    UNIT_LEADER_KIND,
    UNIT_LEADER_SOURCES,
};

use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    read,
};
use engine_native::{
    ccharacter,
    cfleet,
    cprovince,
    ctask_force,
    cunit,
    cweather_manager::{self, region_weather as sweather_per_region},
};
use engine_native_parts::{
    cnaval_mission,
    cnavy_leader,
    modifier_accumulator::self as accumulator,
};
use engine_native_memory::modifier_cache::Value;
use crate::cstrategic_navy;
use engine_executable::modifier_keys as keys;
use engine_sites::Rva;

mod convoy_targets;
pub use convoy_targets::{
    ConvoyDesign, ConvoyDesigns, SourceSpeed, TargetOperands, read_source_speed,
};

pub const TASK_FORCE_KEYS: [u32; 9] = [
    keys::NAVAL_COORDINATION,
    keys::NAVAL_SPEED_FACTOR,
    keys::NAVAL_DETECTION,
    keys::SPOTTING_CHANCE,
    keys::NAVAL_RANGE,
    keys::NAVAL_RANGE_FACTOR,
    keys::CONVOY_RAIDING_EFFICIENCY_FACTOR,
    keys::NIGHT_SPOTTING_CHANCE,
    keys::NAVY_VISIBILITY,
];
pub const COUNTRY_KEYS: [u32; 12] = [
    keys::NAVAL_COORDINATION,
    keys::NAVAL_SPEED_FACTOR,
    keys::NAVAL_DETECTION,
    keys::SPOTTING_CHANCE,
    keys::NAVAL_RANGE,
    keys::NAVAL_RANGE_FACTOR,
    keys::CONVOY_RAIDING_EFFICIENCY_FACTOR,
    keys::NIGHT_SPOTTING_CHANCE,
    keys::CONVOY_ESCORT_EFFICIENCY,
    keys::NAVY_VISIBILITY,
    keys::NAVY_WEATHER_PENALTY,
    keys::UNDERWAY_REPLENISHMENT_RANGE_FACTOR,
];

pub const RAIDING: u32 = 3;
pub const ESCORT: u32 = 4;
pub const MISSION_KINDS: [u32; 4] = [2, RAIDING, ESCORT, 9];

pub const HUBS: usize = 0x170;

pub const STAT_RAIDING: usize = 0x1f8;

#[cfg(any(test, feature = "test-support"))]
pub const FLEET_LEADER: usize = cfleet::LEADER;


pub const MISSION_OWNER: usize = 0x10;
pub const MISSION_CONVOYS: usize = 0x40;
pub const MISSION_RANGE_FLEET: usize = 0xf0;
pub const MISSION_RANGE: usize = 0xf8;

pub const PRESENCE: usize = 0x148;
const PRESENCE_ENTRIES: usize = PRESENCE + 0x08;
const PRESENCE_MASK: usize = PRESENCE + 0x14;
const PRESENCE_PROBE: usize = PRESENCE + 0x18;
const PRESENCE_STRIDE: usize = 0x28;
const PRESENCE_HASH: u32 = 0x045d_9f3b;
pub const PRESENCE_WINDOW: usize = 0x2d0;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Operands {
    pub stat_block: usize,
    pub location: usize,
    pub id: u64,
    pub kind: u32,
    pub mission_owner: u32,
    pub region: u32,
    pub raiding_factor: i64,
    pub range_base: i64,
    pub fuel: [i64; 2],
    pub underway: u8,
    pub replenishment: [i32; 2],
    pub expeditionary_owner: i32,
    pub local: [Value; 9],
    pub leader: u64,
    pub headquarters: usize,
    pub range_fleet: u64,
    pub range: [i32; 2],
    pub convoys: i32,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RaidingGrid {
    pub active: u8,
    pub region: Option<u32>,
    pub spotting: Option<i64>,
    pub targets: Option<[Option<bool>; 2]>,
    pub target: TargetOperands,
    pub design: ConvoyDesign,
    pub source_speed: SourceSpeed,
}

const RAID_SPOTTING: usize = 0x80;
const RAID_TARGETS: usize = 0x90;
const RAID_REGION: usize = 0xe8;

pub const RAID_SPOTTING_SAVE_TOKEN: u32 = 0x3bb0;
pub const RAID_REGION_SAVE_TOKEN: u32 = 0x3c83;

pub fn read_raiding_grid(
    memory: &impl Memory,
    base: usize,
    visit: cfleet::TaskForceVisit,
    resolver: &mut engine_native_parts::creference_registry::ScopedResolver,
) -> Option<RaidingGrid> {
    if visit.kind != RAIDING {
        return None;
    }
    let mission = visit.task.checked_add(MISSION)?;
    let active: u8 = read(memory, mission.checked_add(cnaval_mission::IS_IN_REGIONS)?)?;
    if active == 0 {
        return Some(RaidingGrid::default());
    }
    let region = mission
        .checked_add(RAID_REGION)
        .and_then(|at| read::<u64>(memory, at))
        .and_then(|pointer| {
            if pointer == 0 {
                Some(0)
            } else {
                engine_native_parts::cstrategic_region::id(memory, base, pointer as usize)
                    .ok()
                    .flatten()
            }
        });
    let spotting = mission
        .checked_add(RAID_SPOTTING)
        .and_then(|at| read::<i64>(memory, at));
    let resolved = mission
        .checked_add(RAID_TARGETS)
        .and_then(|at| read::<[u8; 16]>(memory, at))
        .map(|bytes| {
            [u64_at(&bytes, 0), u64_at(&bytes, 8)]
                .map(|reference| resolver.resolve(memory, reference))
        });
    let targets =
        resolved.map(|values| values.map(|value| value.ok().map(|pointer| pointer.is_some())));
    let target = resolved
        .map(|values| convoy_targets::target(memory, values))
        .unwrap_or_default();
    Some(RaidingGrid {
        active,
        region,
        spotting,
        targets,
        target,
        design: ConvoyDesign::default(),
        source_speed: SourceSpeed::default(),
    })
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Presence {
    pub positive: i32,
    pub total: i32,
}

pub type Resolve<'a> = &'a dyn Fn(u64) -> Result<Option<usize>, &'static str>;

fn at(pointer: usize, offset: usize) -> Result<usize, &'static str> {
    pointer
        .checked_add(offset)
        .ok_or("naval mission input address overflow")
}

fn field<T: engine_native_memory::memory::Raw>(
    memory: &impl Memory,
    pointer: usize,
    offset: usize,
    what: &'static str,
) -> Result<T, &'static str> {
    read(memory, at(pointer, offset)?).ok_or(what)
}

fn vtable(base: usize, rva: Rva) -> Result<u64, &'static str> {
    Ok(engine_sites::at(base, rva).ok_or("naval mission vtable withheld")? as u64)
}

fn identity(
    memory: &impl Memory,
    base: usize,
    pointer: usize,
    rva: Rva,
    what: &'static str,
) -> Result<(), &'static str> {
    (read::<u64>(memory, pointer) == Some(vtable(base, rva)?))
        .then_some(())
        .ok_or(what)
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(bytes[at..at + 8].try_into().expect("eight bytes"))
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
}

fn i32_at(bytes: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(bytes[at..at + 4].try_into().expect("four bytes"))
}

const MISSION: usize = ctask_force::NAVAL_MISSION;

const TASK_FIELDS: [RecordField; 13] = {
    const fn field(source_offset: usize, destination_offset: usize, len: usize) -> RecordField {
        RecordField {
            source_offset,
            destination_offset,
            len,
        }
    }
    [
        field(MISSION, 112, 16),
        field(MISSION + MISSION_CONVOYS, 0, 4),
        field(cunit::STAT_BLOCK, 8, 8),
        field(cunit::LOCATION, 16, 8),
        field(ctask_force::ATTACHED_HEADQUARTERS, 40, 16),
        field(MISSION + MISSION_RANGE_FLEET, 56, 8),
        field(MISSION + MISSION_RANGE, 64, 8),
        field(MISSION + MISSION_OWNER, 72, 4),
        field(FUEL, 80, 16),
        field(REPLENISHMENT, 96, 8),
        field(UNDERWAY, 104, 1),
        field(cunit::EXPEDITIONARY_OWNER, 108, 4),
        field(LOCAL_MODIFIERS, 128, 16),
    ]
};
const TASK_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&TASK_FIELDS, 144).expect("task force operand layout");

#[cfg(test)]
pub fn read_task_force(
    memory: &impl Memory,
    base: usize,
    visit: cfleet::TaskForceVisit,
    headquarters: &mut Vec<u64>,
) -> Result<Operands, &'static str> {
    read_task_force_observed(memory, base, visit, headquarters, &mut None)
}

#[cfg(test)]
pub fn read_task_force_observed(
    memory: &impl Memory,
    base: usize,
    visit: cfleet::TaskForceVisit,
    headquarters: &mut Vec<u64>,
    original_header: &mut Option<Result<[u8; 16], &'static str>>,
) -> Result<Operands, &'static str> {
    read_task_force_transport_observed(
        memory,
        base,
        visit,
        headquarters,
        original_header,
        &mut None,
    )
}
#[cfg(test)]
pub(crate) fn read_task_force_transport_observed(memory: &impl Memory, base: usize, visit: cfleet::TaskForceVisit, headquarters: &mut Vec<u64>, original_header: &mut Option<Result<[u8; 16], &'static str>>, original_owner: &mut Option<Result<u32, &'static str>>) -> Result<Operands, &'static str> {
    read_task_force_transport_observed_buffer(memory, base, visit, engine_native_memory::walk::Buffer::Growing(headquarters), original_header, original_owner)
}


pub fn read_task_force_transport_observed_buffer(
    memory: &impl Memory,
    base: usize,
    visit: cfleet::TaskForceVisit,
    headquarters: engine_native_memory::walk::Buffer<'_, u64>,
    original_header: &mut Option<Result<[u8; 16], &'static str>>,
    original_owner: &mut Option<Result<u32, &'static str>>,
) -> Result<Operands, &'static str> {
    read_task_force_transport_observed_buffer_with_stats(
        memory,
        base,
        visit,
        headquarters,
        original_header,
        original_owner,
        &mut OriginalSelectors::default(),
    )
}

#[derive(Clone, Copy, Debug, Default)]
pub struct OriginalSelectors {
    pub stats: Option<Result<usize, &'static str>>,
    pub location: Option<Result<usize, &'static str>>,
}

pub fn read_task_force_transport_observed_buffer_with_stats(
    memory: &impl Memory,
    base: usize,
    visit: cfleet::TaskForceVisit,
    mut headquarters: engine_native_memory::walk::Buffer<'_, u64>,
    original_header: &mut Option<Result<[u8; 16], &'static str>>,
    original_owner: &mut Option<Result<u32, &'static str>>,
    original_selectors: &mut OriginalSelectors,
) -> Result<Operands, &'static str> {
    let task = visit.task;
    let mut bytes = [0; 144];
    let copied = memory.copy_records(RecordSources::Pointers(&[task]), &TASK_LAYOUT, &mut bytes);
    *original_header = Some(if copied == 0 {
        Err("transport original mission header unreadable")
    } else {
        Ok(bytes[112..128].try_into().expect("original mission header"))
    });
    original_selectors.stats = if copied >= 3 {
        Some(Ok(u64_at(&bytes, 8) as usize))
    } else if copied == 2 {
        Some(Err("original task-force stat selector unreadable"))
    } else {
        None
    };
    original_selectors.location = if copied >= 4 {
        Some(Ok(u64_at(&bytes, 16) as usize))
    } else if copied == 3 {
        Some(Err("original task-force location selector unreadable"))
    } else {
        None
    };
    *original_owner = if copied >= 8 {
        Some(Ok(u32_at(&bytes, 72)))
    } else if copied == 7 {
        Some(Err("transport original mission owner unreadable"))
    } else {
        None
    };
    if copied != TASK_FIELDS.len() {
        return Err("task force operands unreadable");
    }
    if u64_at(&bytes, 112) != vtable(base, cnaval_mission::VTABLE_RVA)?
        || u64_at(&bytes, 120) != task as u64
    {
        return Err("naval mission identity mismatch");
    }
    let mut row = Operands {
        id: visit.id,
        kind: visit.kind,
        mission_owner: u32_at(&bytes, 72),
        convoys: i32_at(&bytes, 0),
        range: [i32_at(&bytes, 64), i32_at(&bytes, 68)],
        leader: visit.leader,
        fuel: [u64_at(&bytes, 80) as i64, u64_at(&bytes, 88) as i64],
        replenishment: [i32_at(&bytes, 96), i32_at(&bytes, 100)],
        underway: bytes[104],
        expeditionary_owner: i32_at(&bytes, 108),
        ..Operands::default()
    };
    let stats = u64_at(&bytes, 8) as usize;
    if stats == 0 {
        return Err("task-force stat block null");
    }
    let mut range = [0; 8];
    let mut raiding = [0; 8];
    if memory.copy_scattered(
        [at(stats, STAT_RANGE)?, at(stats, STAT_RAIDING)?],
        &mut [&mut range, &mut raiding],
    ) != 2
    {
        return Err("task-force range or raiding stat unreadable");
    }
    row.stat_block = stats;
    row.range_base = i64::from_le_bytes(range);
    row.raiding_factor = i64::from_le_bytes(raiding);
    let location = u64_at(&bytes, 16) as usize;
    row.location = location;
    if location != 0 {
        let mut province_vtable = [0; 8];
        let mut region_bytes = [0; 8];
        let copied = memory.copy_scattered(
            [location, at(location, cprovince::STRATEGIC_REGION)?],
            &mut [&mut province_vtable, &mut region_bytes],
        );
        if copied == 0
            || u64::from_le_bytes(province_vtable) != vtable(base, cprovince::VTABLE_RVA)?
        {
            return Err("task-force location is not a province");
        }
        if copied < 2 {
            return Err("task-force location unreadable");
        }
        let region = u64::from_le_bytes(region_bytes);
        row.region = engine_native_parts::cstrategic_region::id(memory, base, region as usize)?
            .ok_or("task-force location has no strategic region")?;
        let local_empty = Array::from_header(
            bytes[128..144].try_into().expect("local modifier header"),
            engine_native_parts::modifier_entry::STRIDE,
        )
        .is_some_and(|array| array.count == 0);
        if !local_empty {
            accumulator::lookup_many_at(
                memory,
                at(task, LOCAL_MODIFIERS)?,
                &TASK_FORCE_KEYS,
                &mut row.local,
            )?;
        }
    }
    let attached = Array::from_header(bytes[40..56].try_into().expect("header"), 8)
        .ok_or("task-force headquarters invalid")?;
    row.headquarters = attached.count as usize;
    headquarters.ensure(row.headquarters)?;
    if let Some(out) = headquarters.get_mut(..row.headquarters) {
        engine_native_memory::cpdx_array::pointers(
            memory,
            attached,
            "task-force headquarters entry unreadable or null",
            |index, entry| {
                if entry == 0 {
                    return Err("task-force headquarters entry unreadable or null");
                }
                let commander: u64 = field(
                    memory,
                    entry,
                    HEADQUARTERS_COMMANDER,
                    "headquarters commander unreadable",
                )?;
                out[index as usize] = if commander == 0 {
                    0
                } else {
                    field(
                        memory,
                        commander as usize,
                        COMMANDER_CHARACTER,
                        "headquarters character unreadable",
                    )?
                };
                Ok(())
            },
        )?;
    }
    let range_fleet = u64_at(&bytes, 56) as usize;
    if range_fleet == visit.fleet {
        row.range_fleet = visit.fleet_handle;
    } else if range_fleet != 0 {
        let mut identity = [0; 8];
        let mut handle = [0; 8];
        if memory.copy_scattered(
            [range_fleet, at(range_fleet, cfleet::HANDLE)?],
            &mut [&mut identity, &mut handle],
        ) != 2
        {
            return Err("mission range fleet unreadable");
        }
        if u64::from_le_bytes(identity) != vtable(base, cfleet::VTABLE_RVA)? {
            return Err("mission range fleet vtable mismatch");
        }
        row.range_fleet = u64::from_le_bytes(handle);
    }
    Ok(row)
}

pub fn ledger_regions(
    memory: &impl Memory,
    navy: usize,
    visit: impl FnMut(u32, Array) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    let outer = Array::read(
        memory,
        at(navy, cstrategic_navy::REGION_MISSIONS)?,
        cstrategic_navy::REGION_MISSIONS_STRIDE,
    )
    .ok_or("navy ledger region array invalid")?;
    ledger_regions_in(memory, outer, visit)
}

pub fn ledger_regions_in(
    memory: &impl Memory,
    outer: Array,
    mut visit: impl FnMut(u32, Array) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    const STRIDE: usize = cstrategic_navy::REGION_MISSIONS_STRIDE;
    const BATCH: usize = 128;
    let mut bytes = [0; BATCH * STRIDE];
    for first in (0..outer.count).step_by(BATCH) {
        let count = ((outer.count - first) as usize).min(BATCH);
        let address = outer
            .element(first, STRIDE)
            .ok_or("navy ledger header address overflow")?;
        if !memory.copy(address, &mut bytes[..count * STRIDE]) {
            return Err("navy ledger region headers unreadable");
        }
        for index in 0..count {
            let header = bytes[index * STRIDE..index * STRIDE + 16]
                .try_into()
                .expect("header");
            let ledger = Array::from_header(header, 8).ok_or("navy region ledger invalid")?;
            if ledger.count != 0 {
                visit(first + index as u32, ledger)?;
            }
        }
    }
    Ok(())
}

pub fn ledger(
    memory: &impl Memory,
    missions: Array,
    out: &mut [usize],
) -> Result<usize, &'static str> {
    let count = missions.count as usize;
    if let Some(out) = out.get_mut(..count) {
        engine_native_memory::cpdx_array::pointers(
            memory,
            missions,
            "navy ledger mission unreadable or null",
            |index, mission| {
                if mission == 0 {
                    return Err("navy ledger mission unreadable or null");
                }
                out[index as usize] = mission;
                Ok(())
            },
        )?;
    }
    Ok(count)
}

pub fn count_navy_hubs(memory: &impl Memory, navy: usize) -> Result<usize, &'static str> {
    navy_hubs_buffer(memory, navy, engine_native_memory::walk::Buffer::Count)
}

pub fn navy_hubs_buffer(
    memory: &impl Memory,
    navy: usize,
    mut out: engine_native_memory::walk::Buffer<'_, u32>,
) -> Result<usize, &'static str> {
    let array = Array::read(memory, at(navy, HUBS)?, 4).ok_or("navy supply hubs invalid")?;
    let count = array.count as usize;
    out.ensure(count)?;
    if count != 0 && count <= out.len() {
        engine_native_memory::cpdx_array::copy_u32s(
            memory,
            array,
            &mut out[..count],
            "navy supply hub unreadable",
        )?;
    }
    Ok(count)
}

pub fn fleet_leader(
    memory: &impl Memory,
    base: usize,
    leader: u64,
    resolve: Resolve<'_>,
) -> Result<Option<usize>, &'static str> {
    let Some(pointer) = resolve(leader)? else {
        return Ok(None);
    };
    identity(
        memory,
        base,
        pointer,
        cnavy_leader::VTABLE_RVA,
        "fleet leader is not a navy leader",
    )?;
    Ok(Some(pointer))
}

pub fn leader_modifiers(
    memory: &impl Memory,
    base: usize,
    leader: u64,
    resolve: Resolve<'_>,
    out: &mut [Value; 9],
) -> Result<(), &'static str> {
    let Some(pointer) = fleet_leader(memory, base, leader, resolve)? else {
        return Ok(());
    };
    accumulator::lookup_many_at(
        memory,
        at(pointer, LEADER_MODIFIERS)?,
        &TASK_FORCE_KEYS,
        out,
    )?;
    Ok(())
}

pub fn headquarters_sources(
    memory: &impl Memory,
    base: usize,
    character: u64,
    resolve: Resolve<'_>,
) -> Result<Option<(usize, [usize; 4])>, &'static str> {
    let Some(character) = resolve(character)? else {
        return Ok(None);
    };
    let mut character_vtable = [0; 8];
    let mut cached_bytes = [0; 8];
    let mut reference_bytes = [0; 8];
    let copied = memory.copy_scattered(
        [
            character,
            at(character, CHARACTER_UNIT_LEADER)?,
            at(character, CHARACTER_UNIT_LEADER_REFERENCE)?,
        ],
        &mut [
            &mut character_vtable,
            &mut cached_bytes,
            &mut reference_bytes,
        ],
    );
    if copied == 0 || u64::from_le_bytes(character_vtable) != vtable(base, ccharacter::VTABLE_RVA)?
    {
        return Err("headquarters character identity mismatch");
    }
    if copied < 2 {
        return Err("character unit leader unreadable");
    }
    let cached = u64::from_le_bytes(cached_bytes);
    let leader = if cached != 0 {
        Some(cached as usize)
    } else {
        if copied < 3 {
            return Err("character unit leader unreadable");
        }
        let reference = u64::from_le_bytes(reference_bytes);
        if reference == 0 {
            None
        } else {
            resolve(reference)?
        }
    };
    let Some(leader) = leader else {
        return Ok(None);
    };
    let mut kind_bytes = [0; 4];
    let mut source_bytes = [[0; 8]; 4];
    let [s0, s1, s2, s3] = &mut source_bytes;
    let copied = memory.copy_scattered(
        [
            at(leader, UNIT_LEADER_KIND)?,
            at(leader, UNIT_LEADER_SOURCES[0])?,
            at(leader, UNIT_LEADER_SOURCES[1])?,
            at(leader, UNIT_LEADER_SOURCES[2])?,
            at(leader, UNIT_LEADER_SOURCES[3])?,
        ],
        &mut [&mut kind_bytes, s0, s1, s2, s3],
    );
    if copied == 0 {
        return Err("unit leader kind unreadable");
    }
    if i32::from_le_bytes(kind_bytes) != 2 {
        return Ok(None);
    }
    if copied < 5 {
        return Err("unit leader source unreadable");
    }
    let mut sources = [0; 4];
    for (raw, source) in source_bytes.into_iter().zip(&mut sources) {
        let pointer = u64::from_le_bytes(raw);
        if pointer == 0 {
            return Err("unit leader modifier source null");
        }
        *source = usize::try_from(pointer).map_err(|_| "unit leader source unreadable")?;
    }
    Ok(Some((leader, sources)))
}

pub fn headquarters_modifiers(
    memory: &impl Memory,
    base: usize,
    character: u64,
    resolve: Resolve<'_>,
    out: &mut [[Value; 9]; 4],
) -> Result<(), &'static str> {
    let Some((_, sources)) = headquarters_sources(memory, base, character, resolve)? else {
        return Ok(());
    };
    for (source, values) in sources.into_iter().zip(out.iter_mut()) {
        accumulator::lookup_many_at(
            memory,
            at(source, SOURCE_MODIFIERS)?,
            &TASK_FORCE_KEYS,
            values,
        )?;
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresenceMap {
    entries: usize,
    mask: u32,
    end: u32,
}

const PRESENCE_ENTRY_FIELDS: [RecordField; 1] = [RecordField {
    source_offset: 0,
    destination_offset: 0,
    len: PRESENCE_STRIDE,
}];
const PRESENCE_ENTRY_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&PRESENCE_ENTRY_FIELDS, PRESENCE_STRIDE).expect("presence entry layout");

impl PresenceMap {
    pub fn read(memory: &impl Memory, navy: usize) -> Result<Self, &'static str> {
        let mut entries = [0; 8];
        let mut mask = [0; 4];
        let mut probe = [0; 1];
        if memory.copy_scattered(
            [
                at(navy, PRESENCE_ENTRIES)?,
                at(navy, PRESENCE_MASK)?,
                at(navy, PRESENCE_PROBE)?,
            ],
            &mut [&mut entries, &mut mask, &mut probe],
        ) != 3
        {
            return Err("presence map unreadable");
        }
        let entries = u64::from_le_bytes(entries) as usize;
        if entries == 0 {
            return Err("presence map has no entries");
        }
        let mask =
            u32::try_from(i32::from_le_bytes(mask)).map_err(|_| "presence map mask invalid")?;
        let end = u32::from(probe[0])
            .checked_add(1)
            .and_then(|end| end.checked_add(mask))
            .ok_or("presence map end overflow")?;
        Ok(Self { entries, mask, end })
    }

    #[inline]
    fn home(&self, slot: u32) -> u32 {
        let hashed = (slot >> 16 ^ slot).wrapping_mul(PRESENCE_HASH);
        let hashed = (hashed >> 16 ^ hashed).wrapping_mul(PRESENCE_HASH);
        (hashed >> 16 ^ hashed) & self.mask
    }

    #[inline]
    fn entry_address(&self, index: u32) -> Result<usize, &'static str> {
        (index as usize)
            .checked_mul(PRESENCE_STRIDE)
            .and_then(|offset| self.entries.checked_add(offset))
            .ok_or("presence entry address overflow")
    }

    fn entry(
        &self,
        memory: &impl Memory,
        index: u32,
    ) -> Result<[u8; PRESENCE_STRIDE], &'static str> {
        let address = self.entry_address(index)?;
        read(memory, address).ok_or("presence entry unreadable")
    }

    fn count_presence(bytes: &[u8], read_count: usize, steps: usize) -> Presence {
        let mut single = Presence::default();
        for chunk in bytes.chunks_exact(4) {
            let value = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            if value >= 0 {
                single.total += 1;
                single.positive += i32::from(value > 0);
            }
        }
        if read_count == steps {
            return single;
        }
        let full = (steps / read_count) as i32;
        let rem = steps % read_count;
        let mut presence = Presence {
            positive: single.positive * full,
            total: single.total * full,
        };
        for chunk in bytes[..rem * 4].chunks_exact(4) {
            let value = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            if value >= 0 {
                presence.total += 1;
                presence.positive += i32::from(value > 0);
            }
        }
        presence
    }

    fn count_ring(
        &self,
        memory: &impl Memory,
        entry: &[u8; PRESENCE_STRIDE],
    ) -> Result<Presence, &'static str> {
        let data = u64_at(entry, 0x10) as usize;
        let capacity = i32_at(entry, 0x18);
        let head = i32_at(entry, 0x1c);
        let tail = i32_at(entry, 0x20);
        if head == tail {
            return Ok(Presence::default());
        }
        if capacity <= 0 || !(0..capacity).contains(&head) {
            return Err("presence ring index invalid");
        }
        let steps = if (0..capacity).contains(&tail) {
            (tail - head).rem_euclid(capacity) as usize
        } else {
            PRESENCE_WINDOW
        }
        .min(PRESENCE_WINDOW);
        let read_count = steps.min(capacity as usize);
        let first = read_count.min((capacity - head) as usize);
        let second = read_count - first;
        let first_addr = (head as usize)
            .checked_mul(4)
            .and_then(|bytes| data.checked_add(bytes))
            .ok_or("presence ring address overflow")?;
        if second == 0 {
            let mut uninit = [std::mem::MaybeUninit::<u8>::uninit(); PRESENCE_WINDOW * 4];
            let bytes = memory
                .copy_uninit(first_addr, &mut uninit[..read_count * 4])
                .ok_or("presence ring unreadable")?;
            Ok(Self::count_presence(bytes, read_count, steps))
        } else {
            let mut buf = [0_u8; PRESENCE_WINDOW * 4];
            let (part0, part1) = buf[..read_count * 4].split_at_mut(first * 4);
            if memory.copy_scattered([first_addr, data], &mut [part0, part1]) != 2 {
                return Err("presence ring unreadable");
            }
            Ok(Self::count_presence(&buf[..read_count * 4], read_count, steps))
        }
    }

    fn resolve_from_home(
        &self,
        memory: &impl Memory,
        slot: u32,
        mut index: u32,
        mut entry: [u8; PRESENCE_STRIDE],
    ) -> Result<Option<Presence>, &'static str> {
        let mut found = false;
        if entry[4] != 0 {
            let mut distance: u8 = 1;
            loop {
                if u32_at(&entry, 8) == slot {
                    found = true;
                    break;
                }
                distance = distance
                    .checked_add(1)
                    .ok_or("presence probe did not end")?;
                index = index.checked_add(1).ok_or("presence probe overflow")?;
                entry = self.entry(memory, index)?;
                if distance > entry[4] {
                    break;
                }
            }
        }
        if !found || index == self.end {
            return Ok(None);
        }
        self.count_ring(memory, &entry).map(Some)
    }

    pub fn presence(
        &self,
        memory: &impl Memory,
        slot: u32,
    ) -> Result<Option<Presence>, &'static str> {
        let index = self.home(slot);
        let entry = self.entry(memory, index)?;
        self.resolve_from_home(memory, slot, index, entry)
    }

    pub fn presences_batch(
        &self,
        memory: &impl Memory,
        slots: &[u32],
        out: &mut [Option<Presence>],
    ) -> Result<(), &'static str> {
        const BATCH: usize = 32;
        let mut homes = [0_u32; BATCH];
        let mut addrs = [0_usize; BATCH];
        let mut raw = [0_u8; BATCH * PRESENCE_STRIDE];
        let mut first = 0_usize;
        while first < slots.len() {
            let count = (slots.len() - first).min(BATCH);
            let mut batch_ok = true;
            for i in 0..count {
                let index = self.home(slots[first + i]);
                homes[i] = index;
                match self.entry_address(index) {
                    Ok(addr) => addrs[i] = addr,
                    Err(_) => {
                        batch_ok = false;
                        break;
                    }
                }
            }
            let complete = if batch_ok {
                memory.copy_records(
                    RecordSources::Pointers(&addrs[..count]),
                    &PRESENCE_ENTRY_LAYOUT,
                    &mut raw[..count * PRESENCE_STRIDE],
                )
            } else {
                0
            };
            for i in 0..count {
                out[first + i] = if i < complete {
                    let entry: [u8; PRESENCE_STRIDE] = raw
                        [i * PRESENCE_STRIDE..(i + 1) * PRESENCE_STRIDE]
                        .try_into()
                        .expect("presence entry width");
                    self.resolve_from_home(memory, slots[first + i], homes[i], entry)?
                } else {
                    self.presence(memory, slots[first + i])?
                };
            }
            first += count;
        }
        Ok(())
    }
}

pub fn region_weather<M: Memory>(
    memory: &M,
    weather: &cweather_manager::Reader<'_, M>,
    region: u32,
    out: &mut [Value; 9],
) -> Result<(), &'static str> {
    let id = i32::try_from(region).map_err(|_| "weather region id out of range")?;
    let record = weather.movement_record(id, true)?;
    if record == 0 {
        *out = [Value::default(); 9];
        return Ok(());
    }
    accumulator::lookup_many_at(
        memory,
        at(record, sweather_per_region::MODIFIERS)?,
        &TASK_FORCE_KEYS,
        out,
    )
}

#[cfg(test)]
mod tests;
