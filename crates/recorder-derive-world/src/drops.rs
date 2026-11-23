
use recorder_derive_framework::framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer};
use replay_columnar::{
    leaf::Writer as Bytes,
    state::Kind,
    store::{TableView, Value as Held},
};
use replay_data::rows::TableId;
use std::{collections::HashMap, io};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

const BALANCES: [usize; 2] = [2, 3];

const READS: &[Read] = &[Read { table: TableId::Countries, columns: Columns::These(&BALANCES), watch: Watch::Changed, required: false }];

#[derive(Default)]
pub struct Drops {
    held: HashMap<u32, [Option<i64>; 2]>,
    last: Option<i32>,
    living: Vec<u128>,
}

fn balance(table: &TableView<'_>, column: usize, slot: u32) -> Option<i64> {
    match (table.kinds().get(column)?, table.value(column, slot)) {
        (Kind::I64 | Kind::OptionalI64, Held::Number(bits)) => Some(bits as i64),
        _ => None,
    }
}

impl Writer for Drops {
    fn name(&self) -> &'static str {
        "Drops"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::BalanceDrops)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn hourly(&self) -> bool {
        true
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        for key in self.living.drain(..) {
            out.delete(TableId::BalanceDrops, key);
        }
        let countries = hour.table(TableId::Countries);
        let consecutive = !hour.fresh && self.last.is_some_and(|last| last + 1 == hour.hour);
        if !hour.complete(TableId::Countries) {
            self.held.clear();
            self.last = None;
            return Ok(());
        }
        self.last = Some(hour.hour);
        if !consecutive {
            self.held.clear();
            for slot in countries.rows() {
                let Some(key) = countries.key(slot) else { continue };
                self.held.insert(key as u32, BALANCES.map(|column| balance(&countries, column, slot)));
            }
            return Ok(());
        }
        for key in hour.deleted_keys(TableId::Countries) {
            self.held.remove(&(key as u32));
        }
        for slot in hour.written_rows(TableId::Countries, Columns::These(&BALANCES)) {
            let Some(key) = countries.key(slot) else { continue };
            let id = key as u32;
            let now = BALANCES.map(|column| balance(&countries, column, slot));
            if let Some(before) = self.held.insert(id, now) {
                for kind in 0..2 {
                    if let (Some(before), Some(after)) = (before[kind], now[kind])
                        && after < before
                    {
                        let row = (kind as u128) << 64 | u128::from(id) << 32 | u128::from(hour.hour as u32);
                        out.set(TableId::BalanceDrops, row, vec![Value::I64(before), Value::I64(after)]);
                        self.living.push(row);
                    }
                }
            }
        }
        Ok(())
    }

    fn prime(&mut self, hour: &Hour<'_>) {
        let drops = hour.table(TableId::BalanceDrops);
        self.living = drops.rows().filter_map(|slot| drops.key(slot)).collect();
        self.held.clear();
        if !hour.complete(TableId::Countries) {
            self.last = None;
            return;
        }
        self.last = Some(hour.hour);
        let countries = hour.table(TableId::Countries);
        for slot in countries.rows() {
            let Some(key) = countries.key(slot) else { continue };
            self.held.insert(key as u32, BALANCES.map(|column| balance(&countries, column, slot)));
        }
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("Drops' state holds nothing"));
        }
        *self = Self::default();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recorder_derive_framework::framework::Writers;
    use replay_columnar::{manifest::Status, state::Cell, store::Store, unit::Discard};
    use replay_data::rows::TABLE_COUNT;

    fn store() -> Store {
        let mut kinds = vec![Vec::new(); TABLE_COUNT];
        kinds[TableId::Countries.index()] = vec![Kind::U32, Kind::U32, Kind::I64, Kind::I64];
        kinds[TableId::BalanceDrops.index()] = vec![Kind::I64, Kind::I64];
        let mut store = Store::new(kinds, vec![false; TABLE_COUNT]);
        store.begin_run(0);
        store
    }

    fn writers() -> Writers {
        let mut writers = Writers::new(vec![Box::new(Drops::default())]);
        writers.start_run();
        writers
    }

    fn hour(store: &mut Store, writers: &mut Writers, hour: i32, countries: &[(u128, i64, i64)]) {
        store.begin_hour(hour, &[]).unwrap();
        let index = TableId::Countries.index();
        store.status(index, Status::Complete, true, Some(hour), true).unwrap();
        let mut changes = store.table(index).unwrap();
        for &(key, political, command) in countries {
            let slot = match changes.slot(key) {
                Some(slot) => slot,
                None => {
                    let slot = changes.insert(key).unwrap();
                    changes.write(slot, 0, Cell::U32(key as u32)).unwrap();
                    changes.write(slot, 1, Cell::U32(0)).unwrap();
                    slot
                }
            };
            changes.offer(slot, 2, Cell::I64(political)).unwrap();
            changes.offer(slot, 3, Cell::I64(command)).unwrap();
        }
        store.settle();
        writers.hour(1, hour, hour == 0, store).unwrap();
        store.end_hour(&mut Discard).unwrap();
    }

    fn drops(store: &Store) -> Vec<u128> {
        let held = store.view(TableId::BalanceDrops.index());
        let mut keys: Vec<u128> = held.rows().filter_map(|slot| held.key(slot)).collect();
        keys.sort_unstable();
        keys
    }

    const HOURS: [[(u128, i64, i64); 2]; 4] = [[(7, 10, 7), (8, 4, 4)], [(7, 10, 7), (8, 4, 4)], [(7, 5, 7), (8, 4, 4)], [(7, 5, 3), (8, 4, 4)]];

    #[test]
    fn a_drop_in_the_first_hour_after_a_resume_is_a_row() {
        let (mut unbroken, mut writing) = (store(), writers());
        for (at, countries) in HOURS.iter().enumerate() {
            hour(&mut unbroken, &mut writing, at as i32, countries);
        }
        let (mut stopped, mut before) = (store(), writers());
        for (at, countries) in HOURS[..3].iter().enumerate() {
            hour(&mut stopped, &mut before, at as i32, countries);
        }
        assert_eq!(drops(&stopped), [7 << 32 | 2], "the hour 2 drop of political power lives at hour 2");
        let saved = before.save().unwrap();
        let mut resumed = store();
        resumed.resume(stopped.tables().to_vec(), stopped.strings().next()).unwrap();
        let mut after = writers();
        after.load(&saved, &mut resumed).unwrap();
        after.prime(&resumed, 1, 2);
        hour(&mut resumed, &mut after, 3, &HOURS[3]);
        let command_drop = 1_u128 << 64 | 7 << 32 | 3;
        assert_eq!(drops(&unbroken), [command_drop]);
        assert_eq!(drops(&resumed), drops(&unbroken), "the first hour after a resume compares with the hour before it");
    }
}
