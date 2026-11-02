use crate::manifest::{Selection, invalid};
use replay::columns;
use replay_columnar::{
    state::{Cell, Kind, Table},
    walk::State,
};
use replay_data::rows::TableId;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
};

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct Footprint {
    pub keys_bytes: u64,
    pub cells_bytes: u64,
    pub live_slots: u64,
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
    fields_end: usize,
}

impl Layout {
    pub fn new(tables: &[TableId], reverse: usize) -> Self {
        let mut dimensions = 0;
        let offsets = tables
            .iter()
            .map(|&id| {
                let at = dimensions;
                dimensions += 1 + id.schema().len();
                (id, at)
            })
            .collect();
        Self {
            tables: tables.into(),
            offsets,
            dimensions: dimensions + reverse,
            previous: BTreeMap::new(),
            fields_end: dimensions,
        }
    }

    pub fn indexes(&self, selections: &[Selection]) -> Vec<usize> {
        let mut indexes = Vec::new();
        for selected in selections {
            let at = self.offsets[&selected.table];
            indexes.push(at);
            indexes.extend(selected.fields.iter().map(|field| at + 1 + field));
        }
        indexes.sort_unstable();
        indexes.dedup();
        indexes
    }

    pub fn field_dimensions(&self) -> usize {
        self.fields_end
    }

    pub fn hold_previous(&mut self, selections: &[Selection]) {
        for at in self.indexes(selections) {
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
            layout,
        }
    }

    pub fn observe(&mut self, state: &State, restore: bool) -> io::Result<()> {
        for meter in &mut self.meters {
            let table = columns::table(state, meter.id)?;
            let edits = columns::changes(state, meter.id)?;
            let live = table.live().count() as u64;
            self.live.insert(meter.id, live);
            self.held_peak
                .entry(meter.id)
                .and_modify(|n| *n = (*n).max(table.held_bytes() as u64))
                .or_insert(table.held_bytes() as u64);
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
                self.values[offset + 1 + field] = live * kind.slot_bytes() as u64
                    + meter.payload[field]
                    + live.div_ceil(64) * 8 * (1 + u64::from(optional));
            }
            meter.initialized = true;
        }
        self.dictionary_peak = self
            .dictionary_peak
            .max(state.dictionary().held_bytes() as u64);
        Ok(())
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
                .map(|f| self.values[at + 1 + f])
                .sum::<u64>();
        }
        footprint.bytes = footprint.keys_bytes + footprint.cells_bytes;
        footprint
    }
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
}
