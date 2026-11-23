
use recorder_derive_framework::{
    forces::{invalid, written},
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns},
};
use replay_columnar::{
    leaf::Writer as Bytes,
    store::{TableView, Value as Held},
};
use replay_data::rows::{AirWingRow, TableId};
use std::{
    collections::{HashMap, HashSet},
    io,
};

const REGION: [usize; 1] = columns::<AirWingRow, 1>(["mission_region"]);

const READS: &[Read] = &[Read { table: TableId::AirWings, columns: Columns::These(&REGION), watch: Watch::Changed, required: true }];

fn wing(view: &TableView<'_>, slot: u32) -> io::Result<(u64, Option<u32>)> {
    let id = view.key(slot).ok_or_else(|| invalid(format!("AirWings slot {slot} holds no row")))? as u64;
    match view.value(REGION[0], slot) {
        Held::Number(region) => Ok((id, Some(region as u32))),
        Held::Missing => Ok((id, None)),
        _ => Err(invalid("AirWings.mission_region is not recorded and its row cannot say so")),
    }
}

#[derive(Default)]
pub struct AirRegionWings {
    rebuild: bool,
    regions: HashMap<u64, u32>,
    counts: HashMap<u32, i64>,
    dirty: HashSet<u32>,
}

impl AirRegionWings {
    fn count(&mut self, region: u32, sign: i64) {
        let count = self.counts.entry(region).or_default();
        *count += sign;
        if *count == 0 {
            self.counts.remove(&region);
        }
        self.dirty.insert(region);
    }

    fn set(&mut self, id: u64, region: Option<u32>) {
        let before = match region {
            Some(region) => self.regions.insert(id, region),
            None => self.regions.remove(&id),
        };
        if before == region {
            return;
        }
        if let Some(before) = before {
            self.count(before, -1);
        }
        if let Some(region) = region {
            self.count(region, 1);
        }
    }
}

impl Writer for AirRegionWings {
    fn name(&self) -> &'static str {
        "AirRegionWings"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::AirRegionWings)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let table = TableId::AirRegionWings;
        let whole = hour.fresh || self.rebuild;
        let view = hour.table(TableId::AirWings);
        if whole {
            self.regions.clear();
            self.counts.clear();
            self.rebuild = false;
            for slot in view.rows() {
                let (id, region) = wing(&view, slot)?;
                self.set(id, region);
            }
        } else {
            for key in hour.deleted_keys(TableId::AirWings) {
                self.set(key as u64, None);
            }
            for slot in written(hour, TableId::AirWings, &REGION) {
                let (id, region) = wing(&view, slot)?;
                self.set(id, region);
            }
        }
        let mut regions: Vec<u32> = std::mem::take(&mut self.dirty).into_iter().collect();
        if whole {
            regions.extend(recorder_derive_framework::forces::held(out, table).into_iter().map(|key| key as u32));
        }
        regions.sort_unstable();
        regions.dedup();
        for region in regions {
            match self.counts.get(&region) {
                Some(&wings) => out.set(table, u128::from(region), vec![Value::I64(wings)]),
                None if out.table(table).slot(u128::from(region)).is_some() => out.delete(table, u128::from(region)),
                None => {}
            }
        }
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("AirRegionWings' state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
