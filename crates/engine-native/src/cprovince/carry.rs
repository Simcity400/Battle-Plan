use super::*;
use crate::cprovince::controller_areas::Pointers;

#[derive(Clone, Copy, Debug, Default)]
struct Held {
    pointer: usize,
    row: Row,
    areas: Pointers,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub carried: u64,
    pub read: u64,
    pub refreshed: u64,
    pub quiet: bool,
    pub full: bool,
}

#[derive(Debug, Default)]
pub struct RowStore {
    epochs: Option<[u64; 2]>,
    header: Array,
    held: Vec<Held>,
    marks: Vec<u32>,
    state_marks: Vec<u32>,
    modifier_marks: Vec<u32>,
    modifier_take: Option<u64>,
    bunkers: Option<u64>,
    countries: Vec<u8>,
    fresh_countries: Vec<u8>,
    carried: Vec<u32>,
    counts: Counts,
    bytes: usize,
    pub force_full: bool,
}

impl RowStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn marks_mut(&mut self) -> (&mut Vec<u32>, &mut Vec<u32>) {
        (&mut self.marks, &mut self.state_marks)
    }

    pub fn modifier_marks_mut(&mut self) -> (&mut Vec<u32>, &mut Option<u64>) {
        (&mut self.modifier_marks, &mut self.modifier_take)
    }

    pub fn forget(&mut self) {
        self.epochs = None;
        self.bunkers = None;
        self.marks.clear();
        self.state_marks.clear();
        self.modifier_marks.clear();
        self.modifier_take = None;
        self.carried.clear();
    }

    pub fn holds(&self) -> bool {
        self.epochs.is_some()
    }

    pub fn counts(&self) -> Counts {
        self.counts
    }

    pub fn allocated_bytes(&self) -> usize {
        self.bytes
    }

    fn measure(&self) -> usize {
        self.held.capacity() * size_of::<Held>()
            + (self.marks.capacity()
                + self.state_marks.capacity()
                + self.modifier_marks.capacity()
                + self.carried.capacity())
                * size_of::<u32>()
            + self.countries.capacity()
            + self.fresh_countries.capacity()
    }

    pub(super) fn refresh_areas(
        &mut self,
        memory: &impl Memory,
        rows: &[Row],
        areas: &mut [Pointers],
        keep: &dyn Fn(u32) -> bool,
    ) -> Result<(), &'static str> {
        let mut at = [0_usize; BATCH_ROWS];
        let mut pointers = [0_usize; BATCH_ROWS];
        let mut raw = [0_u8; BATCH_ROWS * AREA_BYTES];
        let mut next = 0;
        while next < self.carried.len() {
            let mut count = 0;
            while count < BATCH_ROWS && next < self.carried.len() {
                let index = self.carried[next] as usize;
                next += 1;
                let (Some(row), Some(current)) = (rows.get(index), areas.get(index)) else {
                    return Err("carried province row out of range");
                };
                if keep(row.id) {
                    at[count] = index;
                    pointers[count] = current.province;
                    count += 1;
                }
            }
            if count == 0 {
                continue;
            }
            if memory.copy_records(
                RecordSources::Pointers(&pointers[..count]),
                &AREA_LAYOUT,
                &mut raw[..count * AREA_BYTES],
            ) != count * AREA_FIELDS.len()
            {
                return Err("province movement fields unreadable");
            }
            for (slot, record) in raw[..count * AREA_BYTES]
                .chunks_exact(AREA_BYTES)
                .enumerate()
            {
                let word = |offset: usize| {
                    u64::from_le_bytes(record[offset..offset + 8].try_into().unwrap()) as usize
                };
                let definition = Some(word(0))
                    .filter(|&definition| definition != 0)
                    .ok_or("province definition pointer invalid")?;
                let fresh = Pointers {
                    province: pointers[slot],
                    definition,
                    controller: word(8),
                    owner: word(16),
                };
                areas[at[slot]] = fresh;
                if let Some(held) = self.held.get_mut(rows[at[slot]].id as usize) {
                    held.areas = fresh;
                }
            }
        }
        Ok(())
    }

    pub(super) fn carried_rows(&self) -> usize {
        self.carried.len()
    }
}

const AREA_FIELDS: [RecordField; 2] = [
    RecordField {
        source_offset: DEFINITION,
        destination_offset: 0,
        len: 8,
    },
    RecordField {
        source_offset: CONTROLLER_AREA,
        destination_offset: 8,
        len: 16,
    },
];
const AREA_BYTES: usize = 24;
const AREA_LAYOUT: RecordLayout<'static> =
    RecordLayout::new(&AREA_FIELDS, AREA_BYTES).expect("valid record layout");

pub(super) struct Hour<'a> {
    pub store: &'a mut RowStore,
    pub take: Option<[u64; 2]>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn walk(
    hour: &mut Hour<'_>,
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    array: Array,
    output: &mut [Row],
    areas: &mut [Pointers],
    conflicts: &mut ConflictBuffers<'_>,
    conflict: &mut ConflictState,
    states: &mut StateCache,
) -> Outcome {
    let take = hour.take;
    let store = &mut *hour.store;
    let required = array.count as usize - 1;
    store.counts = Counts::default();
    store.carried.clear();
    for marks in [
        &mut store.marks,
        &mut store.state_marks,
        &mut store.modifier_marks,
    ] {
        marks.sort_unstable();
        marks.dedup();
    }
    let modifiers = store.modifier_take.take();
    let witnessed = modifiers.is_some()
        && crate::ccountry::copy_links_and_roster(memory, game_state, &mut store.fresh_countries);
    let bunkers =
        witnessed && store.bunkers == modifiers && store.countries == store.fresh_countries;
    let force_full = std::mem::take(&mut store.force_full);
    let eligible = !force_full
        && take.is_some()
        && take == store.epochs
        && store.header == array
        && store.held.len() == array.count as usize
        && output.len() >= required
        && areas.len() >= required
        && engine_allocation::advisory(|| engine_allocation::reserve(&mut store.carried, required))
            .is_ok();
    let outcome = if eligible {
        carry(
            store, memory, base, game_state, array, output, areas, conflicts, conflict, states,
            bunkers,
        )
    } else {
        let outcome = read_rows(
            memory, base, game_state, array, output, areas, conflicts, conflict, states,
        );
        store.counts.full = true;
        store.counts.read = outcome.len() as u64;
        outcome
    };
    let held = match outcome {
        Outcome::Rows { len } => {
            take.is_some()
                && (eligible
                    || areas
                        .get(..len)
                        .is_some_and(|areas| hold(store, array, &output[..len], areas)))
        }
        _ => false,
    };
    if held {
        store.epochs = take;
        store.header = array;
        store.bunkers = modifiers.filter(|_| witnessed);
        if witnessed {
            std::mem::swap(&mut store.countries, &mut store.fresh_countries);
        }
    } else {
        store.forget();
    }
    store.marks.clear();
    store.state_marks.clear();
    store.modifier_marks.clear();
    store.bytes = store.measure();
    outcome
}

fn hold(store: &mut RowStore, array: Array, rows: &[Row], areas: &[Pointers]) -> bool {
    store.held.clear();
    if engine_allocation::advisory(|| {
        engine_allocation::resize_default(&mut store.held, array.count as usize)
    })
    .is_err()
    {
        return false;
    }
    for (row, areas) in rows.iter().zip(areas) {
        let Some(held) = store.held.get_mut(row.id as usize) else {
            return false;
        };
        *held = Held {
            pointer: areas.province,
            row: Row {
                controller_area: None,
                owner_area: None,
                area_node: None,
                ..*row
            },
            areas: *areas,
        };
    }
    true
}

#[derive(Clone, Copy)]
enum Kind {
    Carried,
    Read {
        body: usize,
    },
}

#[allow(clippy::too_many_arguments)]
fn carry(
    store: &mut RowStore,
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    array: Array,
    output: &mut [Row],
    areas: &mut [Pointers],
    conflicts: &mut ConflictBuffers<'_>,
    conflict: &mut ConflictState,
    states: &mut StateCache,
    bunkers: bool,
) -> Outcome {
    store.counts.quiet = store.marks.is_empty() && store.state_marks.is_empty();
    let mut batch = ProvinceBatch::new();
    let mut movement = MovementBatch::new();
    let mut countries = None;
    let mut bodies = [0_usize; BATCH_ROWS];
    let mut ids = [0_u32; BATCH_ROWS];
    let mut fields = [MovementFields::default(); BATCH_ROWS];
    let mut kinds = [Kind::Carried; BATCH_ROWS];
    let mut provinces = [0_u32; BATCH_ROWS];
    let mut marked = 0;
    let mut first = 1;
    let mut len = 0_usize;
    while first < array.count {
        batch.read_pointers(memory, array, first);
        let admitted = len;
        let mut staged = 0;
        for slot in 0..batch.pointer_count {
            let pointer = batch.pointer(slot);
            let id = first + slot as u32;
            let held = &mut store.held[id as usize];
            if pointer == 0 {
                held.pointer = 0;
                continue;
            }
            while store.marks.get(marked).is_some_and(|&mark| mark < id) {
                marked += 1;
            }
            let carryable = held.pointer == pointer
                && store.marks.get(marked) != Some(&id)
                && (held.row.state == 0
                    || store.state_marks.binary_search(&held.row.state).is_err())
                && held.row.move_gate & Row::MOVE_BORDER_CONFLICT == 0;
            let index = len - admitted;
            provinces[index] = id;
            kinds[index] = if carryable {
                Kind::Carried
            } else {
                bodies[staged] = pointer;
                staged += 1;
                Kind::Read { body: staged - 1 }
            };
            len += 1;
        }
        batch.read_bodies(memory, &bodies[..staged]);
        for index in 0..len - admitted {
            let id = provinces[index];
            output[admitted + index] = match kinds[index] {
                Kind::Carried => store.held[id as usize].row,
                Kind::Read { body, .. } => {
                    match batch.row_body(memory, base, id, bodies[body], body, states) {
                        Ok(row) => {
                            ids[body] = row.id;
                            row
                        }
                        Err(reason) => return Outcome::Unavailable { reason },
                    }
                }
            };
        }
        if staged != 0
            && let Err(reason) = movement.read(
                memory,
                &bodies[..staged],
                &ids[..staged],
                &mut fields[..staged],
            )
        {
            return Outcome::Unavailable { reason };
        }
        for index in 0..len - admitted {
            let at = admitted + index;
            let row = &mut output[at];
            let held = &mut store.held[provinces[index] as usize];
            match kinds[index] {
                Kind::Carried => {
                    let kept = bunkers
                        && row.fort_bunker.is_some()
                        && (row.state == 0
                            || store.modifier_marks.binary_search(&row.state).is_err());
                    if row.fort_positive != 0 && (row.controller as i32) > 0 && !kept {
                        row.fort_bunker = fort_bunker(
                            memory,
                            base,
                            game_state,
                            held.pointer,
                            row.controller,
                            &mut countries,
                        );
                        held.row.fort_bunker = row.fort_bunker;
                        store.counts.refreshed += 1;
                    }
                    areas[at] = held.areas;
                    store.carried.push(at as u32);
                    store.counts.carried += 1;
                }
                Kind::Read { body } => {
                    let (fields, pointer) = (fields[body], bodies[body]);
                    row.fort_positive = u8::from(fields.fort_sum > 0);
                    row.fort_bunker = if fields.fort_sum > 0 && (row.controller as i32) > 0 {
                        fort_bunker(
                            memory,
                            base,
                            game_state,
                            pointer,
                            row.controller,
                            &mut countries,
                        )
                    } else {
                        None
                    };
                    row.move_gate = fields.move_gate;
                    row.port_sea = fields.port_sea;
                    let fresh = Pointers {
                        province: pointer,
                        definition: fields.definition,
                        controller: fields.controller_area,
                        owner: fields.owner_area,
                    };
                    areas[at] = fresh;
                    if fields.move_gate & Row::MOVE_BORDER_CONFLICT != 0 {
                        conflict.record(memory, base, pointer, row.id, conflicts);
                    }
                    *held = Held {
                        pointer,
                        row: *row,
                        areas: fresh,
                    };
                    store.counts.read += 1;
                }
            }
        }
        if batch.pointer_count < batch.len {
            return Outcome::Unavailable {
                reason: if array
                    .element(first + batch.pointer_count as u32, 8)
                    .is_none()
                {
                    "table address overflow"
                } else {
                    "table pointer unreadable"
                },
            };
        }
        first += batch.len as u32;
    }
    if len == 0 {
        Outcome::Unavailable {
            reason: "province table has no readable provinces",
        }
    } else {
        Outcome::Rows { len }
    }
}
