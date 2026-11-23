
use recorder_derive_framework::{
    forces::{invalid, written},
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns},
};
use replay_columnar::leaf::Writer as Bytes;
use replay_data::rows::{TableId, ThreatCountryAggregateRow};
use std::{collections::HashMap, io};

const TOTAL: [usize; 1] = columns::<ThreatCountryAggregateRow, 1>(["total_raw"]);

const READS: &[Read] = &[Read {
    table: TableId::ThreatCountryAggregates,
    columns: Columns::These(&TOTAL),
    watch: Watch::Changed,
    required: true,
}];

#[derive(Default)]
pub struct ThreatTotals {
    rebuild: bool,
    held: HashMap<u32, Option<i64>>,
    sum: i128,
    unknown: usize,
}

impl ThreatTotals {
    fn set(&mut self, slot: u32, value: Option<Option<i64>>) {
        let before = match value {
            Some(value) => self.held.insert(slot, value),
            None => self.held.remove(&slot),
        };
        match before {
            Some(Some(old)) => self.sum -= i128::from(old),
            Some(None) => self.unknown -= 1,
            None => {}
        }
        match value {
            Some(Some(new)) => self.sum += i128::from(new),
            Some(None) => self.unknown += 1,
            None => {}
        }
    }

    fn row(&self) -> Vec<Value> {
        let sum = (self.unknown == 0).then(|| i64::try_from(self.sum).ok()).flatten();
        vec![sum.map_or(Value::Unrecorded, Value::I64)]
    }
}

impl Writer for ThreatTotals {
    fn name(&self) -> &'static str {
        "ThreatTotals"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::ThreatTotals)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let input = TableId::ThreatCountryAggregates;
        let view = hour.table(input);
        let slot_of = |row: u32| view.key(row).map(|key| key as u32).ok_or_else(|| invalid(format!("{input:?} slot {row} holds no row")));
        if hour.fresh || self.rebuild {
            self.rebuild = false;
            self.held.clear();
            self.sum = 0;
            self.unknown = 0;
            for row in view.rows() {
                self.set(slot_of(row)?, Some(view.number(TOTAL[0], row)));
            }
        } else {
            if !hour.wrote(input, Columns::These(&TOTAL)) {
                return Ok(());
            }
            for key in hour.deleted_keys(input) {
                self.set(key as u32, None);
            }
            for row in written(hour, input, &TOTAL) {
                self.set(slot_of(row)?, Some(view.number(TOTAL[0], row)));
            }
        }
        out.set(TableId::ThreatTotals, 0, self.row());
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("ThreatTotals' state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
