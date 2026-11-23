use engine_native_memory::{Memory, read};
use engine_native::{cfleet, ctask_force, cunit};
use engine_native_parts::cnaval_mission;

pub const CHANCE: usize = 0x78;
pub const PROGRESS: usize = 0x80;
pub const TARGET: usize = 0x88;
pub const SURFACE_DETECTION: usize = 0x138;
pub const CARRIER_DETECTION: usize = 0x160;
pub const SUBMARINE_DETECTION: usize = 0x140;
pub const CARRIER_SUBMARINE_DETECTION: usize = 0x158;
pub const SURFACE_VISIBILITY: usize = 0x148;
pub const SUBMARINE_VISIBILITY: usize = 0x150;
pub const FORMATION_CLASS: usize = 0x4b8;
pub const RADAR_CACHE: usize = 0x20;
pub const AIR_CACHE: usize = 0x28;
pub const SAVE_TOKENS: [u32; 3] = [0x3bad, 0x3bb0, 0x3bc4];
pub const CACHE_SAVE_TOKENS: [u32; 2] = [0x2fcd, 0x302f];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Operands {
    pub target: Option<u64>,
    pub target_live: Option<u32>,
    pub progress: Option<i64>,
    pub chance: Option<i64>,
    pub surface_detection: Option<i64>,
    pub carrier_detection: Option<i64>,
    pub submarine_detection: Option<i64>,
    pub carrier_submarine_detection: Option<i64>,
    pub surface_visibility: Option<i64>,
    pub submarine_visibility: Option<i64>,
    pub formation_class: Option<i32>,
    pub radar_cache: Option<i64>,
    pub air_cache: Option<i64>,
}

pub fn read_header(
    memory: &impl Memory,
    base: usize,
    visit: cfleet::TaskForceVisit,
) -> Result<[u8; 16], &'static str> {
    let mission = visit
        .task
        .checked_add(ctask_force::NAVAL_MISSION)
        .ok_or("patrol mission address overflow")?;
    let header = read::<[u8; 16]>(memory, mission).ok_or("patrol mission header unreadable")?;
    authenticate_header(base, visit, &header)?;
    Ok(header)
}
fn authenticate_header(
    base: usize,
    visit: cfleet::TaskForceVisit,
    header: &[u8; 16],
) -> Result<(), &'static str> {
    let identity = u64::from_le_bytes(header[..8].try_into().expect("mission identity"));
    let task = u64::from_le_bytes(header[8..].try_into().expect("mission backlink"));
    if Some(identity) != engine_sites::at(base, cnaval_mission::VTABLE_RVA).map(|at| at as u64)
        || task != visit.task as u64
    {
        return Err("patrol mission identity or backlink invalid");
    }
    Ok(())
}
pub fn read_owner(
    memory: &impl Memory,
    base: usize,
    visit: cfleet::TaskForceVisit,
    header: Result<[u8; 16], &'static str>,
) -> Result<u32, &'static str> {
    authenticate_header(base, visit, &header?)?;
    visit
        .task
        .checked_add(ctask_force::NAVAL_MISSION)
        .and_then(|at| at.checked_add(crate::naval_mission_inputs::MISSION_OWNER))
        .and_then(|at| read(memory, at))
        .ok_or("patrol mission owner unreadable")
}

pub fn read_member_observed_stats(
    memory: &impl Memory,
    base: usize,
    visit: cfleet::TaskForceVisit,
    original_header: Option<Result<[u8; 16], &'static str>>,
    original_stats: Option<Result<usize, &'static str>>,
    convoy_demand: bool,
    resolver: &mut engine_native_parts::creference_registry::ScopedResolver,
) -> Operands {
    read_with(
        memory,
        base,
        visit,
        original_header,
        original_stats,
        convoy_demand,
        |reference| resolver.resolve(memory, reference),
    )
}

fn read_with(
    memory: &impl Memory,
    base: usize,
    visit: cfleet::TaskForceVisit,
    original_header: Option<Result<[u8; 16], &'static str>>,
    original_stats: Option<Result<usize, &'static str>>,
    convoy_demand: bool,
    mut resolve: impl FnMut(u64) -> Result<Option<usize>, &'static str>,
) -> Operands {
    let mut row = Operands::default();
    let word = |pointer: usize, offset: usize| {
        pointer
            .checked_add(offset)
            .filter(|_| pointer != 0)
            .and_then(|at| read::<i64>(memory, at))
    };
    if let Some(stats) = original_stats
        .unwrap_or_else(|| {
            word(visit.task, cunit::STAT_BLOCK)
                .map(|value| value as usize)
                .ok_or("patrol stat selector unreadable")
        })
        .ok()
        .filter(|&value| value != 0)
    {
        row.surface_detection = word(stats, SURFACE_DETECTION);
        row.carrier_detection = word(stats, CARRIER_DETECTION);
        row.submarine_detection = word(stats, SUBMARINE_DETECTION);
        row.carrier_submarine_detection = word(stats, CARRIER_SUBMARINE_DETECTION);
        if convoy_demand {
            row.surface_visibility = word(stats, SURFACE_VISIBILITY);
            row.submarine_visibility = word(stats, SUBMARINE_VISIBILITY);
        }
    }
    if convoy_demand {
        row.formation_class = visit
            .task
            .checked_add(FORMATION_CLASS)
            .and_then(|at| read(memory, at));
    }
    let Some(mission) = visit.task.checked_add(ctask_force::NAVAL_MISSION) else {
        return row;
    };
    let header = original_header.unwrap_or_else(|| {
        read::<[u8; 16]>(memory, mission).ok_or("patrol mission header unreadable")
    });
    let Ok(header) = header else {
        return row;
    };
    let identity = u64::from_le_bytes(header[..8].try_into().expect("mission identity"));
    let owner = u64::from_le_bytes(header[8..].try_into().expect("mission backlink"));
    if Some(identity) != engine_sites::at(base, cnaval_mission::VTABLE_RVA).map(|at| at as u64)
        || owner != visit.task as u64
    {
        return row;
    }
    if convoy_demand {
        row.radar_cache = word(mission, RADAR_CACHE);
        row.air_cache = word(mission, AIR_CACHE);
    }
    row.target = word(mission, TARGET).map(|value| value as u64);
    if let Some(target) = row.target {
        if target == 0 {
            row.target_live = Some(0);
        } else {
            row.target_live = resolve(target)
                .ok()
                .map(|pointer| u32::from(pointer.is_some()));
        }
    }
    if convoy_demand || row.target.is_some_and(|target| target != 0) {
        row.progress = word(mission, PROGRESS);
        row.chance = word(mission, CHANCE);
    }
    row
}

#[derive(Debug, Clone, Copy)]
pub struct Context {
    pub leader: u64,
    pub headquarters: Result<usize, &'static str>,
    pub local_detection: Option<engine_native_parts::modifier_accumulator::Value>,
    pub region: Option<u32>,
}

#[cfg(test)]
pub fn read_context(
    memory: &impl Memory,
    base: usize,
    visit: cfleet::TaskForceVisit,
    headquarters: engine_native_memory::walk::Buffer<'_, u64>,
) -> Context {
    read_context_with_location(memory, base, visit, None, headquarters)
}

pub fn read_context_with_location(
    memory: &impl Memory,
    base: usize,
    visit: cfleet::TaskForceVisit,
    original_location: Option<Result<usize, &'static str>>,
    mut headquarters: engine_native_memory::walk::Buffer<'_, u64>,
) -> Context {
    let mut result = Context {
        leader: visit.leader,
        headquarters: Err("patrol headquarters unreadable"),
        local_detection: None,
        region: None,
    };
    let at = |offset| visit.task.checked_add(offset);
    let location = original_location
        .unwrap_or_else(|| {
            at(cunit::LOCATION)
                .and_then(|at| read::<u64>(memory, at))
                .map(|value| value as usize)
                .ok_or("patrol location selector unreadable")
        })
        .ok();
    if let Some(location) = location {
        if location == 0 {
            result.region = Some(0);
            result.local_detection = Some(Default::default());
        } else {
            result.region = (|| {
                if read::<u64>(memory, location)
                    != engine_sites::at(base, engine_native::cprovince::VTABLE_RVA).map(|at| at as u64)
                {
                    return None;
                }
                let region = location
                    .checked_add(engine_native::cprovince::STRATEGIC_REGION)
                    .and_then(|at| read::<u64>(memory, at))?;
                engine_native_parts::cstrategic_region::id(memory, base, region as usize)
                    .ok()
                    .flatten()
            })();
            let mut value = [engine_native_parts::modifier_accumulator::Value::default(); 1];
            result.local_detection = at(crate::naval_mission_inputs::LOCAL_MODIFIERS)
                .and_then(|at| {
                    engine_native_parts::modifier_accumulator::lookup_many_at(
                        memory,
                        at,
                        &[engine_executable::modifier_keys::NAVAL_DETECTION],
                        &mut value,
                    )
                    .ok()
                })
                .map(|()| value[0]);
        }
    }
    result.headquarters = (|| {
        let roots = at(ctask_force::ATTACHED_HEADQUARTERS)
            .and_then(|at| engine_native_memory::cpdx_array::Array::read(memory, at, 8))
            .ok_or("patrol headquarters vector invalid")?;
        let count = roots.count as usize;
        headquarters.ensure(count)?;
        let Some(output) = headquarters.get_mut(..count) else {
            return Err("patrol headquarters arena too small");
        };
        engine_native_memory::cpdx_array::pointers(
            memory,
            roots,
            "patrol headquarters reference unreadable",
            |index, pointer| {
                if pointer == 0 {
                    return Err("patrol headquarters entry is null");
                }
                let commander = pointer
                    .checked_add(crate::naval_mission_inputs::HEADQUARTERS_COMMANDER)
                    .and_then(|at| read::<u64>(memory, at))
                    .ok_or("patrol headquarters commander unreadable")?;
                output[index as usize] = if commander == 0 {
                    0
                } else {
                    (commander as usize)
                        .checked_add(crate::naval_mission_inputs::COMMANDER_CHARACTER)
                        .and_then(|at| read::<u64>(memory, at))
                        .ok_or("patrol headquarters character unreadable")?
                };
                Ok(())
            },
        )?;
        Ok(count)
    })();
    result
}

#[cfg(test)]
#[path = "naval_patrol_inputs/tests.rs"]
mod tests;
