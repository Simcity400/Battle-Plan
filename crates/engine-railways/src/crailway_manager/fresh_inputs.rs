
use engine_sites::Rva;
use super::{eta_address, eta_array, eta_building_efficiency, eta_global, eta_global_pointer, eta_key, eta_pointer, eta_ratio, eta_read};
use engine_native_memory::{
    Memory,
    cpdx_array::Array,
    memory::{RecordField, RecordLayout, RecordSources},
    walk::Outcome,
};
use engine_native_parts::{
    cbuilding as building,
    cbuilding_database as database,
    cbuilding_template as template,
};
use engine_native::{cprovince::buildings as status, cstate};
use crate::cprovince_railway_info as info;

type Input<T> = Result<T, &'static str>;
const BATCH: usize = 64;
const NOT_READ: &str = "fresh railway input not acquired";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RailwayBuildingHealth {
    pub level: i16,
    pub healthy: i16,
    pub partial: Option<i64>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RailwayBuildingState {
    pub building: Option<RailwayBuildingHealth>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateFractionRow {
    pub state: u32,
    pub fraction: Input<i64>,
}

impl Default for StateFractionRow {
    fn default() -> Self {
        Self {
            state: 0,
            fraction: Err(NOT_READ),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct RailwaySource {
    pub province: u32,
    pub info: usize,
    pub building_status: usize,
    pub cooldown: Input<i32>,
    pub pairs: Input<Array>,
    pub selection: Option<super::selected_inputs::SelectionRow>,
    pub carried: bool,
}

impl Default for RailwaySource {
    fn default() -> Self {
        Self {
            province: 0,
            info: 0,
            building_status: 0,
            cooldown: Err(NOT_READ),
            pairs: Err(NOT_READ),
            selection: None,
            carried: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RailwayBuildingRow {
    pub province: u32,
    pub building: Input<Option<RailwayBuildingState>>,
    pub selection: Option<super::selected_inputs::SelectionRow>,
}

impl Default for RailwayBuildingRow {
    fn default() -> Self {
        Self {
            province: 0,
            building: Err(NOT_READ),
            selection: None,
        }
    }
}

fn capacity(source: usize, output: usize) -> Outcome {
    let len = source.min(output);
    match u32::try_from(source - len) {
        Ok(0) => Outcome::Rows { len },
        Ok(unread_rows) => Outcome::Truncated { len, unread_rows },
        Err(_) => Outcome::Unavailable {
            reason: "fresh railway row demand overflow",
        },
    }
}

const fn field(source_offset: usize, destination_offset: usize, len: usize) -> RecordField {
    RecordField {
        source_offset,
        destination_offset,
        len,
    }
}

const WORD: RecordLayout<'static> =
    RecordLayout::new(&[field(0, 0, 8)], 8).expect("valid word layout");
const INDEX: RecordLayout<'static> =
    RecordLayout::new(&[field(0, 0, 4)], 4).expect("valid index layout");
const HEADER: RecordLayout<'static> =
    RecordLayout::new(&[field(0, 0, 16)], 16).expect("valid header layout");
const STATE_STATUS: RecordLayout<'static> = RecordLayout::new(
    &[field(0, 0, 8), field(status::BUILDING_INDICES, 8, 16)],
    24,
)
.expect("valid state status layout");
const STATE_BUILDING: RecordLayout<'static> = RecordLayout::new(
    &[
        field(0, 0, 8),
        field(building::BUILDING_STATUS, 8, 8),
        field(building::TEMPLATE, 16, 8),
        field(building::TYPE_ID, 24, 4),
        field(building::HEALTHY_LEVEL, 28, 2),
    ],
    30,
)
.expect("valid infrastructure building layout");
const RAIL_POINTER: RecordLayout<'static> =
    RecordLayout::new(&[field(info::BUILDING, 0, 8)], 8).expect("valid rail pointer layout");
const RAIL_BUILDING: RecordLayout<'static> = RecordLayout::new(
    &[
        field(0, 0, 8),
        field(building::BUILDING_STATUS, 8, 8),
        field(building::LEVEL, 16, 2),
    ],
    18,
)
.expect("valid rail health layout");
const RAIL_HEALTHY: RecordLayout<'static> =
    RecordLayout::new(&[field(building::HEALTHY_LEVEL, 0, 2)], 2)
        .expect("valid rail healthy layout");
const RAIL_PARTIAL: RecordLayout<'static> =
    RecordLayout::new(&[field(building::PARTIAL_HEALTH, 0, 8)], 8)
        .expect("valid rail partial layout");

fn word(bytes: &[u8]) -> u64 {
    u64::from_le_bytes(bytes[..8].try_into().unwrap())
}
fn signed(bytes: &[u8]) -> i32 {
    i32::from_le_bytes(bytes[..4].try_into().unwrap())
}
fn pointer(bytes: &[u8]) -> Input<usize> {
    usize::try_from(word(bytes)).map_err(|_| "fresh railway pointer overflow")
}

struct Window {
    pointers: [usize; BATCH],
    rows: [usize; BATCH],
    completed: [usize; BATCH],
    len: usize,
    bytes: [u8; BATCH * 30],
}

impl Window {
    fn new() -> Self {
        Self {
            pointers: [0; BATCH],
            rows: [0; BATCH],
            completed: [0; BATCH],
            len: 0,
            bytes: [0; BATCH * 30],
        }
    }
    fn push(&mut self, row: usize, pointer: usize) {
        self.rows[self.len] = row;
        self.pointers[self.len] = pointer;
        self.len += 1;
    }
    fn gather(&mut self, memory: &impl Memory, layout: &RecordLayout<'_>) {
        self.completed.fill(0);
        let fields = layout.field_count();
        let mut start = 0;
        while start < self.len {
            let copied = memory.copy_records(
                RecordSources::Pointers(&self.pointers[start..self.len]),
                layout,
                &mut self.bytes[start * layout.stride()..self.len * layout.stride()],
            );
            let complete_rows = copied / fields;
            self.completed[start..start + complete_rows].fill(fields);
            start += complete_rows;
            if start < self.len {
                self.completed[start] = copied % fields;
                start += 1;
            }
        }
    }
    fn row(&self, index: usize, stride: usize) -> &[u8] {
        &self.bytes[index * stride..(index + 1) * stride]
    }
    fn completed(&self, index: usize) -> usize {
        self.completed[index]
    }
    fn clear(&mut self) {
        self.pointers.fill(0);
        self.len = 0;
    }
}

#[derive(Clone, Copy)]
struct Definition {
    database: usize,
    template: usize,
    enabled: bool,
}

#[derive(Default)]
struct Controls {
    definition: Option<Input<Definition>>,
    ordinal: Option<Input<u32>>,
    token: Option<Input<u32>>,
    maximum: Option<Input<i32>>,
}

fn once<T: Copy>(cache: &mut Option<Input<T>>, read: impl FnOnce() -> Input<T>) -> Input<T> {
    *cache.get_or_insert_with(read)
}

impl Controls {
    fn definition(&mut self, memory: &impl Memory, base: usize) -> Input<Definition> {
        once(&mut self.definition, || {
            let database = eta_global_pointer(memory, base, database::SINGLETON_RVA)?;
            let template = eta_pointer(memory, database, 0x3b0)?;
            let enabled =
                eta_read::<u8>(memory, template, template::LOOKUP_ENABLED_CANDIDATE)? != 0;
            Ok(Definition {
                database,
                template,
                enabled,
            })
        })
    }
    fn ordinal(&mut self, memory: &impl Memory, definition: Definition) -> Input<u32> {
        once(&mut self.ordinal, || {
            let ordinal = eta_read::<u32>(memory, definition.template, template::DATABASE_ORDINAL)?;
            let templates = eta_array(memory, definition.database, database::TEMPLATES, 8)?;
            let at = templates
                .element(ordinal, 8)
                .ok_or("ETA infrastructure template identity mismatch")?;
            if eta_read::<u64>(memory, at, 0)? != definition.template as u64 {
                return Err("ETA infrastructure template identity mismatch");
            }
            Ok(ordinal)
        })
    }
    fn token(&mut self, memory: &impl Memory, definition: Definition) -> Input<u32> {
        once(&mut self.token, || {
            eta_read(memory, definition.template, template::TYPE_ID)
        })
    }
    fn maximum(&mut self, memory: &impl Memory, base: usize, definition: Definition) -> Input<i32> {
        once(&mut self.maximum, || {
            let preferred = eta_read::<i32>(memory, definition.template, 0x2bc)?;
            if preferred > 0 {
                return Ok(preferred);
            }
            let fallback = eta_read::<i32>(memory, definition.template, 0x2b8)?;
            if fallback > 0 {
                return Ok(fallback);
            }
            eta_global(memory, base, Rva(0x033e_5274))
        })
    }
}

pub fn state_fractions(
    memory: &impl Memory,
    base: usize,
    sources: &[cstate::Source],
    output: &mut [StateFractionRow],
) -> Outcome {
    state_fractions_carried(memory, base, sources, output, |_| None)
}

pub fn state_fractions_carried(
    memory: &impl Memory,
    base: usize,
    sources: &[cstate::Source],
    output: &mut [StateFractionRow],
    mut carried: impl FnMut(usize) -> Option<Input<i64>>,
) -> Outcome {
    output.fill(StateFractionRow::default());
    let outcome = capacity(sources.len(), output.len());
    if matches!(outcome, Outcome::Unavailable { .. }) {
        return outcome;
    }
    let mut controls = Controls::default();
    for (batch, (sources, output)) in sources[..outcome.len()]
        .chunks(BATCH)
        .zip(output[..outcome.len()].chunks_mut(BATCH))
        .enumerate()
    {
        for (row, source) in output.iter_mut().zip(sources) {
            row.state = source.id;
        }
        let definition = match controls.definition(memory, base) {
            Ok(definition) => definition,
            Err(reason) => {
                for row in output {
                    row.fraction = Err(reason);
                }
                continue;
            }
        };
        let mut current = Window::new();
        for (index, source) in sources.iter().enumerate() {
            if let Some(fraction) = carried(batch * BATCH + index) {
                output[index].fraction = fraction;
            } else if source.id == 0 || source.pointer == 0 {
                output[index].fraction = Err("ETA infrastructure state identity mismatch");
            } else if !definition.enabled {
                output[index].fraction = Ok(0);
            } else {
                match eta_address(source.pointer, cstate::BUILDING_STATUS) {
                    Ok(status) => current.push(index, status),
                    Err(reason) => output[index].fraction = Err(reason),
                }
            }
        }
        let mut next = Window::new();
        current.gather(memory, &STATE_STATUS);
        for index in 0..current.len {
            let row = current.rows[index];
            let result = (|| {
                let fields = current.completed(index);
                if fields == 0 {
                    return Err("ETA infrastructure status unreadable");
                }
                let bytes = current.row(index, 24);
                if word(bytes) != eta_key(base, status::VTABLE_RVA)? as u64 {
                    return Err("ETA railway source vtable mismatch");
                }
                if fields < 2 {
                    return Err("ETA infrastructure status unreadable");
                }
                let indices = Array::from_header(bytes[8..24].try_into().unwrap(), 4)
                    .ok_or("ETA railway array invalid")?;
                if indices.count == 0 {
                    output[row].fraction = Ok(0);
                    return Ok(());
                }
                let ordinal = controls.ordinal(memory, definition)?;
                let at = indices
                    .element(ordinal, 4)
                    .ok_or("ETA infrastructure index absent")?;
                next.push(row, at);
                Ok(())
            })();
            if let Err(reason) = result {
                output[row].fraction = Err(reason);
            }
        }
        current.clear();
        std::mem::swap(&mut current, &mut next);
        let mut indices = [0_i32; BATCH];
        current.gather(memory, &INDEX);
        for index in 0..current.len {
            let row = current.rows[index];
            if current.completed(index) == 0 {
                output[row].fraction = Err("ETA infrastructure index unreadable");
                continue;
            }
            let selected = signed(current.row(index, 4));
            if selected == -1 {
                output[row].fraction = Ok(0);
            } else if selected < 0 {
                output[row].fraction = Err("ETA infrastructure index negative");
            } else {
                indices[row] = selected;
                let at = eta_address(sources[row].pointer, cstate::BUILDING_STATUS)
                    .and_then(|status| eta_address(status, status::BUILDINGS));
                match at {
                    Ok(at) => next.push(row, at),
                    Err(reason) => output[row].fraction = Err(reason),
                }
            }
        }
        current.clear();
        std::mem::swap(&mut current, &mut next);
        current.gather(memory, &HEADER);
        for index in 0..current.len {
            let row = current.rows[index];
            let result = (|| {
                if current.completed(index) == 0 {
                    return Err("ETA infrastructure buildings unreadable");
                }
                let buildings = Array::from_header(current.row(index, 16).try_into().unwrap(), 8)
                    .ok_or("ETA railway array invalid")?;
                let at = buildings
                    .element(indices[row] as u32, 8)
                    .ok_or("ETA infrastructure building absent")?;
                next.push(row, at);
                Ok(())
            })();
            if let Err(reason) = result {
                output[row].fraction = Err(reason);
            }
        }
        current.clear();
        std::mem::swap(&mut current, &mut next);
        current.gather(memory, &WORD);
        for index in 0..current.len {
            let row = current.rows[index];
            let result = (|| {
                if current.completed(index) == 0 {
                    return Err("ETA infrastructure building pointer unreadable");
                }
                let instance = pointer(current.row(index, 8))?;
                if instance == 0 {
                    output[row].fraction = Ok(0);
                } else {
                    next.push(row, instance);
                }
                Ok(())
            })();
            if let Err(reason) = result {
                output[row].fraction = Err(reason);
            }
        }
        current.clear();
        std::mem::swap(&mut current, &mut next);
        current.gather(memory, &STATE_BUILDING);
        for index in 0..current.len {
            let row = current.rows[index];
            output[row].fraction = (|| {
                let fields = current.completed(index);
                if fields == 0 {
                    return Err("ETA infrastructure building unreadable");
                }
                let bytes = current.row(index, 30);
                if word(bytes) != eta_key(base, building::VTABLE_RVA)? as u64 {
                    return Err("ETA railway source vtable mismatch");
                }
                if fields < 2 {
                    return Err("ETA infrastructure building unreadable");
                }
                if word(&bytes[8..])
                    != eta_address(sources[row].pointer, cstate::BUILDING_STATUS)? as u64
                {
                    return Err("ETA infrastructure building identity mismatch");
                }
                if fields < 3 {
                    return Err("ETA infrastructure building unreadable");
                }
                if word(&bytes[16..]) != definition.template as u64 {
                    return Err("ETA infrastructure building identity mismatch");
                }
                if fields < 4 {
                    return Err("ETA infrastructure building unreadable");
                }
                if u32::from_le_bytes(bytes[24..28].try_into().unwrap())
                    != controls.token(memory, definition)?
                {
                    return Err("ETA infrastructure building identity mismatch");
                }
                if fields < 5 {
                    return Err("ETA infrastructure building unreadable");
                }
                let healthy = i16::from_le_bytes(bytes[28..30].try_into().unwrap());
                let maximum = controls.maximum(memory, base, definition)?;
                eta_ratio(i64::from(healthy) * 100_000, i64::from(maximum) * 100_000)
            })();
        }
        current.clear();
        next.clear();
    }
    outcome
}

pub(super) fn railway_buildings(
    memory: &impl Memory,
    base: usize,
    sources: &[RailwaySource],
    output: &mut [RailwayBuildingRow],
) -> Outcome {
    output.fill(RailwayBuildingRow::default());
    let outcome = capacity(sources.len(), output.len());
    if matches!(outcome, Outcome::Unavailable { .. }) {
        return outcome;
    }
    for (sources, output) in sources[..outcome.len()]
        .chunks(BATCH)
        .zip(output[..outcome.len()].chunks_mut(BATCH))
    {
        let mut current = Window::new();
        let mut next = Window::new();
        let mut eta_demand = [false; BATCH];
        let mut selected_demand = [false; BATCH];
        for (index, source) in sources.iter().enumerate() {
            output[index].province = source.province;
            output[index].selection = source.selection;
            if source.carried {
                continue;
            }
            match (|| Ok(source.cooldown? <= 0 && source.pairs?.count != 0))() {
                Ok(true) => eta_demand[index] = true,
                Ok(false) => output[index].building = Ok(None),
                Err(reason) => output[index].building = Err(reason),
            }
            if let Some(selected) = &mut output[index].selection {
                let demand = (|| {
                    if source.cooldown? != 0 {
                        return Ok(false);
                    }
                    if selected.pair_count.is_ok_and(|count| count != 0)
                        || selected.levels.is_ok_and(|levels| levels.positive)
                    {
                        return Ok(true);
                    }
                    selected.pair_count?;
                    selected.levels?;
                    Ok(false)
                })();
                match demand {
                    Ok(true) => selected_demand[index] = true,
                    Ok(false) => selected.building_level = Ok(None),
                    Err(reason) => selected.building_level = Err(reason),
                }
            }
            if eta_demand[index] || selected_demand[index] {
                current.push(index, source.info);
            }
        }
        current.gather(memory, &RAIL_POINTER);
        for index in 0..current.len {
            let row = current.rows[index];
            let result = (|| {
                if current.completed(index) == 0 {
                    return Err("fresh railway building pointer unreadable");
                }
                let instance = pointer(current.row(index, 8))?;
                if instance == 0 {
                    if eta_demand[row] {
                        output[row].building = Ok(Some(RailwayBuildingState { building: None }));
                    }
                    if selected_demand[row] {
                        output[row].selection.as_mut().unwrap().building_level = Ok(Some(None));
                    }
                } else {
                    next.push(row, instance);
                }
                Ok(())
            })();
            if let Err(reason) = result {
                building_failure(
                    &mut output[row],
                    eta_demand[row],
                    selected_demand[row],
                    reason,
                );
            }
        }
        current.clear();
        std::mem::swap(&mut current, &mut next);
        current.gather(memory, &RAIL_BUILDING);
        for index in 0..current.len {
            let row = current.rows[index];
            let result = (|| {
                let fields = current.completed(index);
                if fields == 0 {
                    return Err("fresh railway building health unreadable");
                }
                let bytes = current.row(index, 18);
                if word(bytes) != eta_key(base, building::VTABLE_RVA)? as u64 {
                    return Err("ETA railway source vtable mismatch");
                }
                if fields < 2 {
                    return Err("fresh railway building health unreadable");
                }
                if word(&bytes[8..]) != sources[row].building_status as u64 {
                    return Err("ETA rail building backlink mismatch");
                }
                if fields < 3 {
                    return Err("fresh railway building health unreadable");
                }
                let level = i16::from_le_bytes(bytes[16..18].try_into().unwrap());
                if selected_demand[row] {
                    output[row].selection.as_mut().unwrap().building_level = Ok(Some(Some(level)));
                }
                if eta_demand[row] {
                    output[row].building = Ok(Some(RailwayBuildingState {
                        building: Some(RailwayBuildingHealth {
                            level,
                            healthy: 0,
                            partial: None,
                        }),
                    }));
                    next.push(row, current.pointers[index]);
                }
                Ok(())
            })();
            if let Err(reason) = result {
                building_failure(
                    &mut output[row],
                    eta_demand[row],
                    selected_demand[row],
                    reason,
                );
            }
        }
        current.clear();
        std::mem::swap(&mut current, &mut next);
        current.gather(memory, &RAIL_HEALTHY);
        for index in 0..current.len {
            let row = current.rows[index];
            if current.completed(index) == 0 {
                output[row].building = Err("fresh railway building health unreadable");
                continue;
            }
            if let Ok(Some(RailwayBuildingState {
                building: Some(health),
            })) = &mut output[row].building
            {
                health.healthy = i16::from_le_bytes(current.row(index, 2).try_into().unwrap());
                if health.level != 0 && health.level != health.healthy {
                    next.push(row, current.pointers[index]);
                }
            }
        }
        current.clear();
        std::mem::swap(&mut current, &mut next);
        current.gather(memory, &RAIL_PARTIAL);
        for index in 0..current.len {
            let row = current.rows[index];
            if current.completed(index) == 0 {
                output[row].building = Err("fresh railway partial health unreadable");
                continue;
            }
            if let Ok(Some(RailwayBuildingState {
                building: Some(health),
            })) = &mut output[row].building
            {
                health.partial = Some(i64::from_le_bytes(
                    current.row(index, 8).try_into().unwrap(),
                ));
            }
        }
        current.clear();
        next.clear();
    }
    outcome
}

fn building_failure(row: &mut RailwayBuildingRow, eta: bool, selected: bool, reason: &'static str) {
    if eta {
        row.building = Err(reason);
    }
    if selected {
        row.selection.as_mut().unwrap().building_level = Err(reason);
    }
}

pub fn building_efficiency(health: RailwayBuildingHealth) -> Input<i64> {
    let partial = if health.level != 0 && health.level != health.healthy {
        health
            .partial
            .ok_or("fresh railway partial health unavailable")?
    } else {
        0
    };
    eta_building_efficiency(health.level, health.healthy, partial)
}

pub fn directed_percentage(
    cooldown: i32,
    edge_level: Option<i32>,
    health: Option<RailwayBuildingState>,
    maximum: i32,
) -> Input<i64> {
    let weighted = match edge_level.filter(|_| cooldown <= 0) {
        None => 0,
        Some(level) => {
            let health = health.ok_or("fresh railway building health unavailable")?;
            let efficiency = match health.building {
                None => 0,
                Some(health) => building_efficiency(health)?,
            };
            i64::from(level).wrapping_mul(efficiency)
        }
    };
    let denominator = i64::from(maximum) * 100_000;
    if denominator == 0 {
        Ok(0)
    } else {
        eta_ratio(weighted, denominator)
    }
}

pub fn road_or_rail_speed(infrastructure: i64, railway: i64, defines: [i64; 4]) -> i64 {
    let [road_base, road_scale, rail_base, rail_scale] = defines;
    let road = road_base.wrapping_add(infrastructure.wrapping_mul(road_scale) / 100_000);
    let rail = rail_base.wrapping_add(railway.wrapping_mul(rail_scale) / 100_000);
    road.max(rail)
}

#[cfg(test)]
mod tests;
