use crate::manifest::{Selection, invalid};
use replay::columns;
use replay_columnar::{
    section::Changes,
    state::{Cell, Kind, Table},
    walk::State,
};
use replay_data::rows::TableId;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
};

/// The bytes `live` cells of a `kind` column hold, a list's or opaque cell's items aside:
/// bools one bit each, lists and opaque cells one `u32` end each in their page.
fn cells_bytes(kind: Kind, live: u64) -> u64 {
    match kind {
        Kind::Bool => live.div_ceil(64) * 8,
        Kind::U8 | Kind::I8 => live,
        Kind::U16 | Kind::I16 => live * 2,
        Kind::U32
        | Kind::I32
        | Kind::Bytes
        | Kind::OptionalI32
        | Kind::OptionalU32
        | Kind::U32List
        | Kind::U64List
        | Kind::Opaque => live * 4,
        Kind::U64 | Kind::I64 | Kind::OptionalI64 | Kind::OptionalU64 => live * 8,
    }
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Footprint {
    pub keys_bytes: u64,
    pub cells_bytes: u64,
    pub index_bytes: u64,
    pub changes_bytes: u64,
    pub live_slots: u64,
    pub allocated_slots: u64,
    pub bytes: u64,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Peak {
    pub restore: Footprint,
    pub peak: Footprint,
    pub peak_hour: i32,
    pub growth_bytes: u64,
}

impl Peak {
    pub fn new(value: Footprint, hour: i32) -> Self {
        Self {
            restore: value.clone(),
            peak: value,
            peak_hour: hour,
            growth_bytes: 0,
        }
    }

    pub fn observe(&mut self, value: Footprint, hour: i32) {
        if value.bytes > self.peak.bytes {
            self.peak = value;
            self.peak_hour = hour;
        }
        self.growth_bytes = self.peak.bytes - self.restore.bytes;
    }
}

#[derive(Clone)]
pub struct Layout {
    pub tables: Vec<TableId>,
    pub offsets: BTreeMap<TableId, usize>,
    pub dimensions: usize,
    pub previous: BTreeMap<usize, usize>,
    pub reader_tables: BTreeMap<TableId, usize>,
    pub reader_changes: BTreeMap<TableId, usize>,
    pub dictionary: usize,
    pub reader_auxiliary: usize,
    fields_end: usize,
}

impl Layout {
    pub fn new(tables: &[TableId], reverse: usize) -> Self {
        let mut dimensions = 0;
        let offsets = tables
            .iter()
            .map(|&id| {
                let at = dimensions;
                dimensions += 1 + 2 * id.schema().len();
                (id, at)
            })
            .collect();
        let fields_end = dimensions;
        dimensions += reverse;
        let reader_tables = tables
            .iter()
            .map(|&id| {
                let at = dimensions;
                dimensions += 1;
                (id, at)
            })
            .collect();
        let reader_changes = tables
            .iter()
            .map(|&id| {
                let at = dimensions;
                dimensions += 1;
                (id, at)
            })
            .collect();
        let dictionary = dimensions;
        let reader_auxiliary = dimensions + 1;
        Self {
            tables: tables.into(),
            offsets,
            dimensions: dimensions + 2,
            previous: BTreeMap::new(),
            reader_tables,
            reader_changes,
            dictionary,
            reader_auxiliary,
            fields_end,
        }
    }

    pub fn indexes(&self, selections: &[Selection]) -> Vec<usize> {
        let mut indexes = Vec::new();
        for selected in selections {
            let at = self.offsets[&selected.table];
            indexes.push(at);
            for &field in &selected.fields {
                indexes.push(at + 1 + selected.table.schema().len() + field);
                if !selected.recorded_only.contains(&field) {
                    indexes.push(at + 1 + field);
                }
            }
        }
        indexes.sort_unstable();
        indexes.dedup();
        indexes
    }

    pub fn reader_indexes(&self, selections: &[Selection]) -> Vec<usize> {
        selections
            .iter()
            .map(|s| self.reader_tables[&s.table])
            .collect()
    }

    pub fn change_indexes(&self, selections: &[Selection]) -> Vec<usize> {
        selections
            .iter()
            .map(|s| self.reader_changes[&s.table])
            .collect()
    }

    pub fn field_dimensions(&self) -> usize {
        self.fields_end
    }

    pub fn hold_previous(&mut self, selections: &[Selection]) {
        for at in self
            .indexes(selections)
            .into_iter()
            .chain(self.reader_indexes(selections))
        {
            let next = self.dimensions;
            if self.previous.insert(at, next).is_none() {
                self.dimensions += 1;
            }
        }
    }
}

struct Meter {
    id: TableId,
    variable: Vec<Option<Vec<u64>>>,
    payload: Vec<u64>,
    initialized: bool,
}

pub struct Census {
    pub layout: Layout,
    pub values: Vec<u64>,
    pub live: BTreeMap<TableId, u64>,
    meters: Vec<Meter>,
    pub held_peak: BTreeMap<TableId, u64>,
    pub dictionary_peak: u64,
    reader_cells: BTreeMap<TableId, u64>,
    slots: BTreeMap<TableId, u64>,
}

impl Census {
    pub fn new(layout: Layout, kinds: &[Vec<Kind>]) -> Self {
        let meters = layout
            .tables
            .iter()
            .map(|&id| Meter {
                id,
                variable: kinds[id.index()]
                    .iter()
                    .map(|kind| match kind {
                        Kind::U32List | Kind::U64List | Kind::Opaque => Some(Vec::new()),
                        _ => None,
                    })
                    .collect(),
                payload: vec![0; id.schema().len()],
                initialized: false,
            })
            .collect();
        Self {
            values: vec![0; layout.dimensions],
            live: BTreeMap::new(),
            meters,
            held_peak: BTreeMap::new(),
            dictionary_peak: 0,
            reader_cells: BTreeMap::new(),
            slots: BTreeMap::new(),
            layout,
        }
    }

    pub fn observe(&mut self, state: &State, restore: bool) -> io::Result<()> {
        for meter in &mut self.meters {
            let table = columns::table(state, meter.id)?;
            let edits = columns::changes(state, meter.id)?;
            let live = table.live().count() as u64;
            self.live.insert(meter.id, live);
            let held = table.held_bytes() as u64;
            self.held_peak
                .entry(meter.id)
                .and_modify(|n| *n = (*n).max(held))
                .or_insert(held);
            self.values[self.layout.reader_tables[&meter.id]] = held;
            self.values[self.layout.reader_changes[&meter.id]] = changes_bytes(edits);
            self.reader_cells.insert(
                meter.id,
                (0..meter.id.schema().len())
                    .map(|field| table.column(field).unwrap().held_bytes() as u64)
                    .sum(),
            );
            self.slots.insert(meter.id, table.slots() as u64);
            let offset = self.layout.offsets[&meter.id];
            self.values[offset] = live * 20 + live.div_ceil(64) * 8;
            for (field, kind) in table.kinds().enumerate() {
                if let Some(sizes) = &mut meter.variable[field] {
                    if sizes.len() < table.slots() {
                        sizes.resize(table.slots(), 0);
                    }
                    let mut update = |slot: u32| -> io::Result<()> {
                        let at = slot as usize;
                        meter.payload[field] -= sizes[at];
                        sizes[at] = if table.live().get(slot) {
                            variable_bytes(cell(table, field, slot)?)
                        } else {
                            0
                        };
                        meter.payload[field] += sizes[at];
                        Ok(())
                    };
                    if restore || !meter.initialized {
                        for slot in table.live().ones() {
                            update(slot)?;
                        }
                    } else {
                        for &slot in &edits.deleted {
                            update(slot)?;
                        }
                        for &slot in &edits.inserted {
                            update(slot)?;
                        }
                        for (&slot, &mask) in edits
                            .changed
                            .iter()
                            .zip(&edits.masks)
                            .chain(edits.fill_slots.iter().zip(&edits.fill_masks))
                        {
                            if mask & (1_u64 << field) != 0 {
                                update(slot)?;
                            }
                        }
                    }
                }
                let optional = matches!(
                    kind,
                    Kind::OptionalI32 | Kind::OptionalU32 | Kind::OptionalI64 | Kind::OptionalU64
                );
                self.values[offset + 1 + field] = cells_bytes(kind, live)
                    + meter.payload[field]
                    + live.div_ceil(64) * 8 * u64::from(optional);
                self.values[offset + 1 + meter.id.schema().len() + field] = live.div_ceil(64) * 8;
            }
            meter.initialized = true;
        }
        self.values[self.layout.dictionary] = state.dictionary().held_bytes() as u64;
        self.values[self.layout.reader_auxiliary] = state.held_bytes() as u64
            - self
                .layout
                .reader_tables
                .values()
                .map(|&at| self.values[at])
                .sum::<u64>();
        self.dictionary_peak = self
            .dictionary_peak
            .max(state.dictionary().held_bytes() as u64);
        Ok(())
    }

    pub fn reader_footprint(&self, selected: &[Selection]) -> Footprint {
        let mut value = Footprint::default();
        for selection in selected {
            let id = selection.table;
            value.bytes += self.values[self.layout.reader_tables[&id]];
            value.changes_bytes += self.values[self.layout.reader_changes[&id]];
            value.cells_bytes += self.reader_cells[&id];
            value.live_slots += self.live[&id];
            value.allocated_slots += self.slots[&id];
        }
        value.keys_bytes = value.bytes - value.cells_bytes;
        value.bytes += value.changes_bytes;
        value
    }

    pub fn footprint(&self, selected: &[Selection]) -> Footprint {
        let mut footprint = Footprint::default();
        for selection in selected {
            let at = self.layout.offsets[&selection.table];
            footprint.keys_bytes += self.values[at];
            footprint.live_slots += self.live[&selection.table];
            footprint.cells_bytes += selection
                .fields
                .iter()
                .map(|f| {
                    self.values[at + 1 + selection.table.schema().len() + f]
                        + if selection.recorded_only.contains(f) {
                            0
                        } else {
                            self.values[at + 1 + f]
                        }
                })
                .sum::<u64>();
        }
        footprint.bytes = footprint.keys_bytes + footprint.cells_bytes;
        footprint
    }
}

fn changes_bytes(changes: &Changes) -> u64 {
    let Changes {
        truncated: _,
        read,
        changed,
        masks,
        written,
        deleted,
        deleted_keys,
        inserted,
        touched,
        filled,
        fill_slots,
        fill_masks,
    } = changes;
    [read, changed, deleted, inserted, touched, fill_slots]
        .into_iter()
        .map(|items| items.capacity() as u64)
        .sum::<u64>()
        * size_of::<u32>() as u64
        + [masks, written, fill_masks]
            .into_iter()
            .map(|items| items.capacity() as u64 * size_of::<u64>() as u64)
            .sum::<u64>()
        + deleted_keys.capacity() as u64 * size_of::<u128>() as u64
        + filled.capacity() as u64 * size_of::<usize>() as u64
}

pub fn cell(table: &Table, field: usize, slot: u32) -> io::Result<Cell<'_>> {
    table
        .column(field)
        .and_then(|c| c.cell(slot))
        .ok_or_else(|| invalid(format!("Live slot {slot} has no column {field} cell")))
}

fn variable_bytes(cell: Cell<'_>) -> u64 {
    match cell {
        Cell::U32List(items) => items.len() as u64 * 4,
        Cell::U64List(items) => items.len() as u64 * 8,
        Cell::Opaque(items) => items.len() as u64,
        _ => 0,
    }
}

pub struct Trace<W> {
    writer: W,
    previous: Vec<u64>,
}

impl<W: Write> Trace<W> {
    pub fn new(mut writer: W, dimensions: usize) -> io::Result<Self> {
        writer.write_all(&(dimensions as u64).to_le_bytes())?;
        Ok(Self {
            writer,
            previous: vec![u64::MAX; dimensions],
        })
    }

    pub fn snapshot(&mut self, hour: i32, values: &[u64]) -> io::Result<()> {
        self.writer.write_all(&hour.to_le_bytes())?;
        let changed = self
            .previous
            .iter()
            .zip(values)
            .filter(|(a, b)| a != b)
            .count();
        self.writer.write_all(&(changed as u32).to_le_bytes())?;
        for (at, (&old, &now)) in self.previous.iter().zip(values).enumerate() {
            if old == now {
                continue;
            }
            self.writer.write_all(&(at as u32).to_le_bytes())?;
            self.writer.write_all(&now.to_le_bytes())?;
        }
        self.previous.copy_from_slice(values);
        Ok(())
    }

    pub fn finish(mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

pub fn read_trace(
    mut reader: impl Read,
    dimensions: usize,
    mut visit: impl FnMut(i32, &[u64]) -> io::Result<()>,
) -> io::Result<()> {
    let mut width = [0; 8];
    reader.read_exact(&mut width)?;
    if u64::from_le_bytes(width) != dimensions as u64 {
        return Err(invalid("Census dimensions changed"));
    }
    let mut values = vec![0; dimensions];
    loop {
        let mut header = [0; 8];
        if reader.read(&mut header[..1])? == 0 {
            break;
        }
        reader.read_exact(&mut header[1..])?;
        let hour = i32::from_le_bytes(header[..4].try_into().unwrap());
        let count = u32::from_le_bytes(header[4..].try_into().unwrap());
        for _ in 0..count {
            let mut item = [0; 12];
            reader.read_exact(&mut item)?;
            let at = u32::from_le_bytes(item[..4].try_into().unwrap()) as usize;
            *values
                .get_mut(at)
                .ok_or_else(|| invalid("Census dimension outside schema"))? =
                u64::from_le_bytes(item[4..].try_into().unwrap());
        }
        visit(hour, &values)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use replay_columnar::{
        manifest::Status,
        restart::{self, Acquisition},
        state::{Bits, Values},
    };

    #[test]
    fn unknown_cells_keep_their_decode_base_and_lists_grow() {
        let schema = replay::columnar_frames::schema().unwrap();
        let id = TableId::Ships;
        let mut table = Table::with_rows(
            &schema[id.index()],
            1,
            Bits::from_words(vec![1]).unwrap(),
            &[99],
        )
        .unwrap();
        let field = columns::field(id, "strength").unwrap();
        table.set(field, &[0], Values::I64(&[123])).unwrap();
        table.unrecord(field, &[0]).unwrap();
        assert!(!table.recorded(field, 0));
        assert_eq!(cell(&table, field, 0).unwrap(), Cell::I64(123));
        let bytes = restart::write(&table, Status::Complete, &Acquisition::default(), &[]).unwrap();
        let mut state = State::new(schema.clone()).unwrap();
        state.restore(id.index(), &bytes).unwrap();
        let layout = Layout::new(&[id], 0);
        let mut census = Census::new(layout, &schema);
        census.observe(&state, true).unwrap();
        assert!(census.values.iter().sum::<u64>() > 0);
    }

    #[test]
    fn reader_allocation_retains_dead_slots_and_complete_columns() {
        let schema = replay::columnar_frames::schema().unwrap();
        let id = TableId::Ships;
        let mut words = vec![0; 65];
        words[64] = 1;
        let mut table = Table::with_rows(
            &schema[id.index()],
            4097,
            Bits::from_words(words).unwrap(),
            &[99],
        )
        .unwrap();
        let field = columns::field(id, "strength").unwrap();
        table.set(field, &[4096], Values::I64(&[123])).unwrap();
        let bytes = restart::write(&table, Status::Complete, &Acquisition::default(), &[]).unwrap();
        let mut state = State::new(schema.clone()).unwrap();
        state.restore(id.index(), &bytes).unwrap();
        let selected = Selection {
            table: id,
            fields: vec![field],
            recorded_only: Default::default(),
            undeclared: false,
            subscription: "lookup".into(),
            previous: true,
            read_rule: None,
        };
        let mut layout = Layout::new(&[id], 1);
        layout.hold_previous(std::slice::from_ref(&selected));
        let reader = layout.reader_tables[&id];
        let previous = layout.previous[&reader];
        assert_ne!(reader, layout.field_dimensions());
        let mut census = Census::new(layout.clone(), &schema);
        census.observe(&state, true).unwrap();
        let allocation = census.reader_footprint(std::slice::from_ref(&selected));
        assert_eq!(
            allocation.bytes,
            columns::table(&state, id).unwrap().held_bytes() as u64
        );
        assert_eq!(allocation.allocated_slots, 4097);
        assert_eq!(allocation.live_slots, 1);
        assert!(allocation.bytes > census.footprint(&[selected]).bytes);
        assert_eq!(
            allocation.keys_bytes + allocation.cells_bytes + allocation.changes_bytes,
            allocation.bytes
        );
        assert_eq!(census.values[previous], 0);
        assert_eq!(
            census.values[reader] + census.values[layout.reader_auxiliary],
            state.held_bytes() as u64
        );
    }

    #[test]
    fn fill_change_capacity_stays_charged_after_a_quiet_hour() {
        use replay_columnar::{
            manifest::{Manifest, Section},
            section::{self, Op, Run, TableHour},
        };

        let schema = replay::columnar_frames::schema().unwrap();
        let id = TableId::Ships;
        let field = columns::field(id, "strength").unwrap();
        let table = Table::with_rows(
            &schema[id.index()],
            64,
            Bits::from_words(vec![u64::MAX]).unwrap(),
            &(0..64).collect::<Vec<_>>(),
        )
        .unwrap();
        let restart =
            restart::write(&table, Status::Complete, &Acquisition::default(), &[]).unwrap();
        let mut state = State::new(schema.clone()).unwrap();
        state.restore(id.index(), &restart).unwrap();
        let selected = Selection {
            table: id,
            fields: vec![field],
            recorded_only: Default::default(),
            undeclared: false,
            subscription: "stream".into(),
            previous: true,
            read_rule: None,
        };
        let mut layout = Layout::new(&[id], 0);
        layout.hold_previous(std::slice::from_ref(&selected));
        assert!(!layout.previous.contains_key(&layout.reader_changes[&id]));
        let mut census = Census::new(layout, &schema);
        census.observe(&state, true).unwrap();
        let before = census.reader_footprint(std::slice::from_ref(&selected));
        assert_eq!(before.changes_bytes, 0);
        let fill = section::write(
            &schema[id.index()],
            &TableHour {
                read: None,
                changed: &[],
                deleted: &[],
                inserted: &[],
                keys: &[],
                inserts: &[],
                runs: &[Run {
                    column: field,
                    op: Op::Fill,
                    positions: &[],
                    values: Some(Values::I64(&[9])),
                    except: &[],
                    patches: None,
                }],
                unrecorded: &[],
                touched: &[],
            },
        )
        .unwrap();
        let mut columns = vec![0; schema[id.index()].len().div_ceil(64)];
        columns[field / 64] |= 1_u64 << (field % 64);
        let mut manifest = Manifest {
            branch: 1,
            hour: 1,
            flags: 0,
            total_ns: 0,
            pool_bytes: 0,
            dropped_frames: 0,
            appends: Vec::new(),
            statuses: vec![Status::Complete; schema.len()],
            read: vec![0; schema.len().div_ceil(64)],
            samples: Vec::new(),
            sections: vec![Section {
                table: id.index(),
                bytes: fill.len() as u64,
                columns,
                ops: replay_columnar::manifest::FILLS,
            }],
        };
        state.apply(&manifest, |_| Ok(fill.as_slice())).unwrap();
        census.observe(&state, false).unwrap();
        let filled = census.reader_footprint(std::slice::from_ref(&selected));
        assert!(filled.changes_bytes >= 64 * (4 + 8));
        assert_eq!(filled.bytes - before.bytes, filled.changes_bytes);
        manifest.hour += 1;
        manifest.sections.clear();
        state.apply(&manifest, |_| unreachable!()).unwrap();
        assert!(columns::changes(&state, id).unwrap().fill_slots.is_empty());
        census.observe(&state, false).unwrap();
        let quiet = census.reader_footprint(&[selected]);
        assert_eq!(quiet.changes_bytes, filled.changes_bytes);
        assert_eq!(quiet.bytes, filled.bytes);
    }

    #[test]
    fn trace_preserves_growth_and_shrink_at_each_hour() {
        let mut bytes = Vec::new();
        let mut trace = Trace::new(&mut bytes, 2).unwrap();
        trace.snapshot(1, &[2, 8]).unwrap();
        trace.snapshot(2, &[10, 0]).unwrap();
        trace.snapshot(3, &[3, 0]).unwrap();
        trace.finish().unwrap();
        let mut rows = Vec::new();
        read_trace(bytes.as_slice(), 2, |h, v| {
            rows.push((h, v.to_vec()));
            Ok(())
        })
        .unwrap();
        assert_eq!(rows, [(1, vec![2, 8]), (2, vec![10, 0]), (3, vec![3, 0])]);
        assert!(read_trace(&bytes[..bytes.len() - 1], 2, |_, _| Ok(())).is_err());
    }

    #[test]
    fn recorded_only_lists_charge_no_payload_and_share_their_bitmap_with_values() {
        let id = TableId::LandCombatResults;
        let field = columns::field(id, "observers").unwrap();
        let layout = Layout::new(&[id], 0);
        let mut census = Census::new(layout.clone(), &replay::columnar_frames::schema().unwrap());
        let at = layout.offsets[&id];
        census.live.insert(id, 1);
        census.values[at] = 28;
        census.values[at + 1 + field] = 1_000_000;
        census.values[at + 1 + id.schema().len() + field] = 8;
        let bits = Selection {
            table: id,
            fields: vec![field],
            recorded_only: [field].into(),
            undeclared: false,
            subscription: "lookup".into(),
            previous: false,
            read_rule: None,
        };
        let values = Selection {
            recorded_only: Default::default(),
            ..bits.clone()
        };
        assert_eq!(census.footprint(std::slice::from_ref(&bits)).bytes, 36);
        assert_eq!(
            census.footprint(std::slice::from_ref(&values)).bytes,
            1_000_036
        );
        let dimensions: std::collections::BTreeSet<_> =
            layout.indexes(&[bits, values]).into_iter().collect();
        assert_eq!(
            dimensions.iter().map(|&at| census.values[at]).sum::<u64>(),
            1_000_036
        );
    }
}
