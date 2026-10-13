//! Province rows held between hours ([`RowStore`]): the Provinces walk reads
//! only provinces a native writer may have changed, and carries the rest.
//!
//! Every field of a [`Row`] but [`Row::fort_bunker`] has writers hooked by the
//! collector's territory family (`crate::territory_mutations`) or forwarded to
//! it by the building family by location:
//!
//! - `state` and `owner`: the state link `+0xC0` and the state's owner
//!   `CState+0xC8`, read through the link. Province initialization stores the
//!   link and marks every province; state reset zeroes the link over the
//!   state's province list (`+0x18`, count `+0x24`) and the owner. State owner
//!   assignment stores the owner. Those name a state; its provinces are those
//!   whose held row links it, the membership before a reset emptied the list.
//!
//! - `controller` `+0x188`: province controller notification, called per
//!   province by state controller assignment, names its province.
//!
//! - `victory_points` `+0x38`: the victory-point setter, reached from effects
//!   and their applier, and the province reader.
//!
//! - `fort_positive`: the healthy levels `CBuilding+0x42` and active building
//!   vector `+0x1E0`, written by the building family's level assignment,
//!   damage, repair, healthy-level destruction, creation, clearing and loading
//!   detours. Each is forwarded by its status's province location; template
//!   weights are content, loaded once per world.
//!
//! - `move_gate`: the border-conflict set size `+0xB0`, written by insertion
//!   and removal (they name a state: its provinces), and owner area `+0xD8`,
//!   written at run time only by the rebuild, which marks every province;
//!   terrain flags are content.
//!
//! - `port_sea` and definition `+0xB8`: content and construction.
//!
//! `fort_bunker` follows the state's modifier hashes, whose resource-gate
//! rebuild names no province, so every carried row with a positive fort sum and
//! controller reads it again every hour. A province whose held row is in a
//! border conflict is read every hour with its groups, which follow combats no
//! membership mark certifies.
//!
//! A province is carried only when the hour's take returned the epochs of the
//! held complete walk, the table header and province slot pointer are the held
//! ones, neither the province nor its held state is marked, its held row is in
//! no conflict and it is not in the hour's sampled slice. Every other province
//! is read exactly as a full walk reads it. A carryable province of the slice
//! (every one under verification) is read afresh and compared with its held
//! row; a difference poisons the store. A walk that does not complete, or an
//! uncertified take, holds nothing.
use super::*;
use crate::controller_areas::Pointers;

/// One province as the latest complete walk left it.
#[derive(Clone, Copy, Debug, Default)]
struct Held {
    /// Its slot pointer; zero holds nothing.
    pointer: usize,
    /// Without area labels, which the collector's area step sets.
    row: Row,
    areas: Pointers,
}

/// One walk's rows, for status.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Rows published from the store.
    pub carried: u64,
    /// Rows read afresh.
    pub read: u64,
    /// Carried rows whose [`Row::fort_bunker`] was read again.
    pub refreshed: u64,
    /// Carryable rows read afresh and compared with the store.
    pub verified: u64,
    /// Of those, the ones that differed.
    pub mismatches: u64,
    /// Nothing was marked.
    pub quiet: bool,
    /// Every row was read: nothing held certified the walk.
    pub full: bool,
}

/// The held walk, lent to the Provinces job each hour.
#[derive(Debug, Default)]
pub struct RowStore {
    /// The territory take's epochs the held walk was certified under; None
    /// holds nothing.
    epochs: Option<[u64; 2]>,
    header: Array,
    /// By province id (its slot), one per slot of `header`.
    held: Vec<Held>,
    /// The hour's marked province ids and state ids.
    marks: Vec<u32>,
    state_marks: Vec<u32>,
    /// The row indices whose area pointers this walk took from `held`.
    carried: Vec<u32>,
    poisoned: bool,
    counts: Counts,
    /// [`Self::allocated_bytes`] as of the latest walk.
    bytes: usize,
    /// A requested full refresh: the next walk reads every province.
    pub force_full: bool,
}

impl RowStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// The hour's marked province ids and the state ids whose provinces a
    /// writer reached; the next walk consumes them.
    pub fn marks_mut(&mut self) -> (&mut Vec<u32>, &mut Vec<u32>) {
        (&mut self.marks, &mut self.state_marks)
    }

    /// A failed job or an incomplete walk: nothing is held.
    pub fn forget(&mut self) {
        self.epochs = None;
        self.marks.clear();
        self.state_marks.clear();
        self.carried.clear();
    }

    /// A carried row differed from a fresh read: every later walk reads every
    /// province.
    pub fn poison(&mut self) {
        self.poisoned = true;
    }

    pub fn poisoned(&self) -> bool {
        self.poisoned
    }

    /// Whether a complete walk is held.
    pub fn holds(&self) -> bool {
        self.epochs.is_some()
    }

    pub fn counts(&self) -> Counts {
        self.counts
    }

    /// The storage held after the latest walk; constant time, for the
    /// callback that takes it back.
    pub fn allocated_bytes(&self) -> usize {
        self.bytes
    }

    fn measure(&self) -> usize {
        self.held.capacity() * size_of::<Held>()
            + (self.marks.capacity() + self.state_marks.capacity() + self.carried.capacity())
                * size_of::<u32>()
    }

    /// Reads again the definition and area pointers of the carried rows
    /// `keep` selects by province id, into `areas` and the store.
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

    /// The rows of the latest walk whose area pointers came from the store.
    pub(super) fn carried_rows(&self) -> usize {
        self.carried.len()
    }
}

/// The definition and both area pointers of a carried row
/// ([`crate::controller_areas::Pointers`]), as [`MovementBatch::read`]
/// gathers them.
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

/// One walk's view of the store.
pub(super) struct Hour<'a> {
    pub store: &'a mut RowStore,
    /// The territory take's epochs; None when it refused.
    pub take: Option<[u64; 2]>,
    /// The sampled slice, by province id.
    pub covers: &'a dyn Fn(u64) -> bool,
    /// Read every province and compare every carryable one.
    pub verify: bool,
}

/// The walk over the province table `array`, its output already sized:
/// carries what the store certifies and holds what completes.
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
    let (take, covers, verify) = (hour.take, hour.covers, hour.verify);
    let store = &mut *hour.store;
    let required = array.count as usize - 1;
    store.counts = Counts::default();
    store.carried.clear();
    for marks in [&mut store.marks, &mut store.state_marks] {
        marks.sort_unstable();
        marks.dedup();
    }
    let force_full = std::mem::take(&mut store.force_full);
    let eligible = !store.poisoned
        && !force_full
        && take.is_some()
        && take == store.epochs
        && store.header == array
        && store.held.len() == array.count as usize
        && output.len() >= required
        && areas.len() >= required
        && crate::allocation::advisory(|| crate::allocation::reserve(&mut store.carried, required))
            .is_ok();
    let outcome = if eligible {
        carry(
            store, covers, verify, memory, base, game_state, array, output, areas, conflicts,
            conflict, states,
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
    } else {
        store.forget();
    }
    if store.counts.mismatches != 0 {
        store.poisoned = true;
    }
    store.marks.clear();
    store.state_marks.clear();
    store.bytes = store.measure();
    outcome
}

/// Holds a complete full walk: every slot of `array`, its rows by id.
fn hold(store: &mut RowStore, array: Array, rows: &[Row], areas: &[Pointers]) -> bool {
    store.held.clear();
    if crate::allocation::advisory(|| {
        crate::allocation::resize_default(&mut store.held, array.count as usize)
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

/// What one output row of a batch is.
#[derive(Clone, Copy)]
enum Kind {
    Carried,
    /// Read into body `body`; `compare` when the store could have carried it.
    Read {
        body: usize,
        compare: bool,
    },
}

/// [`read_rows`] for an hour the store certifies: the batch's carried rows
/// read nothing but the slot pointers and, when fortified, `fort_bunker`;
/// the others read their bodies in one dense gather and finish as a full
/// walk finishes them. The output holds every slot (`walk` checked).
#[allow(clippy::too_many_arguments)]
fn carry(
    store: &mut RowStore,
    covers: &dyn Fn(u64) -> bool,
    verify: bool,
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
    store.counts.quiet = store.marks.is_empty() && store.state_marks.is_empty();
    let mut batch = ProvinceBatch::new();
    let mut movement = MovementBatch::new();
    // The country census, read once for the walk's first fortified province.
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
                // An emptied slot holds nothing.
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
            kinds[index] = if carryable && !verify && !covers(u64::from(id)) {
                Kind::Carried
            } else {
                bodies[staged] = pointer;
                staged += 1;
                Kind::Read {
                    body: staged - 1,
                    compare: carryable,
                }
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
                    if row.fort_positive != 0 && (row.controller as i32) > 0 {
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
                Kind::Read { body, compare } => {
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
                    if compare {
                        // fort_bunker is read every hour; the rest is what
                        // the carry would have published.
                        store.counts.verified += 1;
                        let carried = Row {
                            fort_bunker: row.fort_bunker,
                            ..held.row
                        };
                        if carried != *row || held.areas.definition != fresh.definition {
                            store.counts.mismatches += 1;
                        }
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
