
use recorder_derive_framework::{
    deployed::{invalid, number, row},
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns},
    trade_flows::{Holders, sides},
};
use replay_columnar::leaf::{Reader, Writer as Bytes};
use replay_data::rows::{
    ConstructionLineOutputRow, ConstructionSpendRow, CountryLinkRow, ProvinceRow, Recorded, TableId, TagHolderRow,
};
use std::{collections::HashMap, io};

#[cfg(test)]
mod tests;

const NONE: u32 = TagHolderRow::NONE;

const OUTPUT: [usize; 11] = columns::<ConstructionLineOutputRow, 11>([
    "railway",
    "construction_raw",
    "repair_raw",
    "since_hour",
    "linked",
    "exact",
    "country",
    "building_type",
    "target_state",
    "route_first",
    "route_last",
]);
const LINK: [usize; 1] = columns::<CountryLinkRow, 1>(["canonical_slot"]);
const PROVINCE_STATE: [usize; 1] = columns::<ProvinceRow, 1>(["state"]);
const TOTAL: usize = 0;
const UNKNOWN: usize = 1;

const READS: &[Read] = &[
    Read { table: TableId::ConstructionLineOutput, columns: Columns::These(&OUTPUT), watch: Watch::Changed, required: true },
    Read { table: TableId::Countries, columns: Columns::These(&[]), watch: Watch::Changed, required: true },
    Read { table: TableId::CountryLinks, columns: Columns::These(&LINK), watch: Watch::Changed, required: true },
    Read { table: TableId::TagHolders, columns: Columns::Every, watch: Watch::Changed, required: true },
    Read { table: TableId::Provinces, columns: Columns::These(&[]), watch: Watch::Changed, required: true },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Line {
    rows: [u128; 2],
    since: i32,
    base: Option<[i64; 2]>,
    held: [i64; 2],
    unknown: bool,
}

type Deltas = HashMap<u128, (i128, i64)>;

fn add(deltas: &mut Deltas, row: u128, amount: i128, unknown: i64) {
    let entry = deltas.entry(row).or_default();
    entry.0 += amount;
    entry.1 += unknown;
}

#[derive(Default)]
pub struct ConstructionSpend {
    lines: HashMap<u64, Line>,
    holders: Holders,
    reread: bool,
}

impl ConstructionSpend {
    fn state_of(hour: &Hour<'_>, id: Option<u32>) -> u32 {
        let Some(id) = id else { return NONE };
        let view = hour.table(TableId::Provinces);
        view.slot(u128::from(id)).and_then(|slot| number(&view, PROVINCE_STATE[0], slot)).map_or(NONE, |state| state as u32)
    }

    fn rows_of(&self, hour: &Hour<'_>, output: &ConstructionLineOutputRow) -> io::Result<[u128; 2]> {
        let known = |cell: &Recorded<Option<u32>>| match cell {
            Recorded::Value(Some(value)) => Some(*value),
            _ => None,
        };
        let (slot, country) = sides(&self.holders, known(&output.country).unwrap_or(0))?;
        let (building_type, state, to_state) = if output.railway {
            (NONE, Self::state_of(hour, known(&output.route_first)), Self::state_of(hour, known(&output.route_last)))
        } else {
            (known(&output.building_type).unwrap_or(NONE), known(&output.target_state).unwrap_or(NONE), NONE)
        };
        let key = |repair| ConstructionSpendRow { slot, country, repair, railway: output.railway, building_type, state, to_state, ..Default::default() }.key();
        Ok([key(false), key(true)])
    }

    fn line(&mut self, hour: &Hour<'_>, slot: u32, deltas: &mut Deltas) -> io::Result<()> {
        let view = hour.table(TableId::ConstructionLineOutput);
        let output: ConstructionLineOutputRow = row(&view, slot, &OUTPUT)?;
        let rows = self.rows_of(hour, &output)?;
        let now = (output.exact && !output.linked).then_some([output.construction_raw, output.repair_raw]);
        let fresh = Line { rows, since: output.since_hour, base: Some([0, 0]), held: [0, 0], unknown: false };
        let mut line = match self.lines.get(&output.line) {
            None => fresh,
            Some(held) if held.since != output.since_hour => fresh,
            Some(held) if held.rows != rows => {
                let line = Line { rows, since: output.since_hour, base: now, held: [0, 0], unknown: now.is_none() };
                if line.unknown {
                    for row in rows {
                        add(deltas, row, 0, 1);
                    }
                }
                line
            }
            Some(held) => *held,
        };
        match (now, line.base) {
            (Some(now), Some(base)) => {
                for at in 0..2 {
                    let counted = now[at].wrapping_sub(base[at]);
                    add(deltas, line.rows[at], i128::from(counted) - i128::from(line.held[at]), if line.unknown { -1 } else { 0 });
                    line.held[at] = counted;
                }
                line.unknown = false;
            }
            (Some(now), None) => {
                line.base = Some(now);
                for row in line.rows {
                    add(deltas, row, 0, -1);
                }
                line.unknown = false;
            }
            (None, _) if !line.unknown => {
                for row in line.rows {
                    add(deltas, row, 0, 1);
                }
                line.unknown = true;
            }
            (None, _) => {}
        }
        self.lines.insert(output.line, line);
        Ok(())
    }

    fn write(deltas: Deltas, out: &mut Rows<'_>) {
        for (row, (amount, unknown)) in deltas {
            if amount == 0 && unknown == 0 {
                continue;
            }
            let view = out.table(TableId::ConstructionSpend);
            let (total, held) = match view.slot(row) {
                Some(slot) => (view.number(TOTAL, slot).map(i128::from), view.number(UNKNOWN, slot).unwrap_or(0)),
                None => (Some(0), 0),
            };
            let total = total.and_then(|total| i64::try_from(total + amount).ok()).map_or(Value::Unrecorded, Value::I64);
            out.set(TableId::ConstructionSpend, row, vec![total, Value::I64(held + unknown)]);
        }
    }
}

impl Writer for ConstructionSpend {
    fn name(&self) -> &'static str {
        "ConstructionSpend"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::ConstructionSpend)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        if hour.fresh || std::mem::take(&mut self.reread) || Holders::changed(hour) {
            self.holders = Holders::read(hour);
        }
        let mut deltas = Deltas::new();
        let view = hour.table(TableId::ConstructionLineOutput);
        let slots: Vec<u32> = if hour.fresh {
            let live: std::collections::HashSet<u64> = view.rows().filter_map(|slot| view.key(slot)).map(|key| key as u64).collect();
            let gone: Vec<u64> = self.lines.keys().copied().filter(|line| !live.contains(line)).collect();
            for line in gone {
                let held = self.lines.remove(&line).expect("held");
                if !held.unknown {
                    for row in held.rows {
                        add(&mut deltas, row, 0, 1);
                    }
                }
            }
            view.rows().collect()
        } else {
            for line in hour.deleted_keys(TableId::ConstructionLineOutput) {
                self.lines.remove(&(line as u64));
            }
            hour.written_rows(TableId::ConstructionLineOutput, Columns::These(&OUTPUT))
        };
        for slot in slots {
            self.line(hour, slot, &mut deltas)?;
        }
        Self::write(deltas, out);
        Ok(())
    }

    fn save(&self, blob: &mut Bytes) -> io::Result<()> {
        let mut lines: Vec<(&u64, &Line)> = self.lines.iter().collect();
        lines.sort_unstable_by_key(|(line, _)| **line);
        blob.varint(lines.len() as u64)?;
        for (id, line) in lines {
            blob.varint(*id)?;
            for row in line.rows {
                blob.raw(&row.to_le_bytes())?;
            }
            blob.raw(&line.since.to_le_bytes())?;
            match line.base {
                None => blob.u8(0)?,
                Some(base) => {
                    blob.u8(1)?;
                    for value in base {
                        blob.raw(&value.to_le_bytes())?;
                    }
                }
            }
            for value in line.held {
                blob.raw(&value.to_le_bytes())?;
            }
            blob.u8(u8::from(line.unknown))?;
        }
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        fn i64_of(reader: &mut Reader<'_>) -> io::Result<i64> {
            Ok(i64::from_le_bytes(reader.raw(8)?.try_into().expect("eight bytes")))
        }
        *self = Self { reread: true, ..Self::default() };
        let mut reader = Reader::new(blob);
        let count = reader.count()?;
        if count > blob.len() {
            return Err(invalid("ConstructionSpend's state counts more lines than it holds"));
        }
        for _ in 0..count {
            let id = reader.varint()?;
            let mut rows = [0; 2];
            for row in &mut rows {
                *row = u128::from_le_bytes(reader.raw(16)?.try_into().expect("sixteen bytes"));
            }
            let since = i32::from_le_bytes(reader.raw(4)?.try_into().expect("four bytes"));
            let base = match reader.u8()? {
                0 => None,
                _ => Some([i64_of(&mut reader)?, i64_of(&mut reader)?]),
            };
            let held = [i64_of(&mut reader)?, i64_of(&mut reader)?];
            let unknown = reader.u8()? != 0;
            self.lines.insert(id, Line { rows, since, base, held, unknown });
        }
        reader.finish()
    }
}
