
use recorder_derive_framework::framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer};
use replay_columnar::{
    leaf::Writer as Bytes,
    state::Kind,
    store::{TableView, Value as Held},
};
use replay_data::{
    country_stability,
    format::FormatVersion,
    rows::{CountryCommandPowerRow, CountryStabilityInputRow, RowCodec, TableId},
};
use std::{
    collections::{HashMap, HashSet},
    io,
};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

const READS: &[Read] = &[
    Read {
        table: TableId::CountryStabilityInputs,
        columns: Columns::Every,
        watch: Watch::Changed,
        required: true,
    },
    Read {
        table: TableId::Countries,
        columns: Columns::These(&[0]),
        watch: Watch::Changed,
        required: true,
    },
    Read {
        table: TableId::CountryCommandPower,
        columns: Columns::Every,
        watch: Watch::Changed,
        required: false,
    },
];

#[derive(Default)]
pub struct Stability {
    rebuild: bool,
    slots: HashMap<u32, u32>,
    ids: HashMap<u32, Vec<u32>>,
    command: bool,
    held: HashSet<u32>,
}

fn u32_cell(table: &TableView<'_>, column: usize, slot: u32) -> Option<u32> {
    match (table.kinds().get(column)?, table.value(column, slot)) {
        (Kind::U32, Held::Number(bits)) => Some(bits as u32),
        _ => None,
    }
}

const INPUT_COLUMNS: u64 = (1 << 34) - 1;

fn row<R: RowCodec>(hour: &Hour<'_>, table: TableId, slot: u32, mask: u64) -> io::Result<R> {
    let held = hour.table(table);
    let key = held.key(slot).ok_or_else(|| invalid(format!("{table:?} slot {slot} holds no row")))?;
    let format = FormatVersion::CURRENT.version();
    R::from_wire_fields(key, mask, &mut |column| replay_frame::frame::field(&held, slot, column, format))
}

fn edited(hour: &Hour<'_>, table: TableId) -> Vec<u128> {
    let held = hour.table(table);
    let slots = hour.written_rows(table, Columns::Every);
    slots.into_iter().filter_map(|slot| held.key(slot)).chain(hour.deleted_keys(table)).collect()
}

fn value(value: Option<i64>) -> Value {
    value.map_or(Value::Unrecorded, Value::I64)
}

fn cells(read: &CountryStabilityInputRow, command: Option<&CountryCommandPowerRow>) -> Vec<Value> {
    vec![
        value(country_stability::stability(read).value),
        value(country_stability::war_support(read).value),
        value(country_stability::max_command_power(read, command).value),
    ]
}

impl Stability {
    fn read_slots(&mut self, hour: &Hour<'_>) -> io::Result<()> {
        let countries = hour.table(TableId::Countries);
        self.slots.clear();
        self.ids.clear();
        for slot in countries.rows() {
            let (Some(key), Some(canonical)) = (countries.key(slot), u32_cell(&countries, 0, slot)) else { continue };
            self.slots.insert(key as u32, canonical);
            self.ids.entry(canonical).or_default().push(key as u32);
        }
        Ok(())
    }

    fn compute(&self, hour: &Hour<'_>, id: u32) -> io::Result<Option<Vec<Value>>> {
        let Some(&canonical) = self.slots.get(&id) else {
            return Ok(None);
        };
        let inputs = hour.table(TableId::CountryStabilityInputs);
        let Some(slot) = inputs.slot(u128::from(id)) else {
            return Ok(None);
        };
        let read: CountryStabilityInputRow = row(hour, TableId::CountryStabilityInputs, slot, INPUT_COLUMNS)?;
        let command: Option<CountryCommandPowerRow> = match self.command {
            true => {
                let held = hour.table(TableId::CountryCommandPower);
                held.slot(u128::from(canonical)).map(|slot| row(hour, TableId::CountryCommandPower, slot, u64::MAX)).transpose()?
            }
            false => None,
        };
        Ok(Some(cells(&read, command.as_ref())))
    }
}

impl Writer for Stability {
    fn name(&self) -> &'static str {
        "Stability"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::CountryStability)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let command = hour.complete(TableId::CountryCommandPower);
        let slots_changed = hour.fresh || self.rebuild || hour.wrote(TableId::Countries, Columns::These(&[0]));
        let mut dirty: HashSet<u32> = HashSet::new();
        if slots_changed || command != self.command {
            self.read_slots(hour)?;
            self.command = command;
            if self.rebuild || hour.fresh {
                self.held.clear();
                let derived = out.table(TableId::CountryStability);
                self.held.extend(derived.rows().filter_map(|slot| derived.key(slot)).map(|key| key as u32));
            }
            self.rebuild = false;
            let inputs = hour.table(TableId::CountryStabilityInputs);
            dirty.extend(inputs.rows().filter_map(|slot| inputs.key(slot)).map(|key| key as u32));
            dirty.extend(self.held.iter().copied());
        } else {
            dirty.extend(edited(hour, TableId::CountryStabilityInputs).into_iter().map(|key| key as u32));
            if command {
                for canonical in edited(hour, TableId::CountryCommandPower) {
                    dirty.extend(self.ids.get(&(canonical as u32)).into_iter().flatten().copied());
                }
            }
        }
        for id in dirty {
            match self.compute(hour, id)? {
                Some(cells) => {
                    out.set(TableId::CountryStability, u128::from(id), cells);
                    self.held.insert(id);
                }
                None => {
                    if self.held.remove(&id) {
                        out.delete(TableId::CountryStability, u128::from(id));
                    }
                }
            }
        }
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("Stability's state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ceiling_without_its_command_power_row_is_not_recorded_never_zero() {
        let read = CountryStabilityInputRow { base_max_command_power: 30_000_000, ..Default::default() };
        let without = cells(&read, None);
        assert_eq!(without[2], Value::Unrecorded);
        let command = CountryCommandPowerRow {
            max_command_power: Some(10_000_000),
            max_command_power_factor: Some(0),
            allocated_command_power_raw: 5_000_000,
            ..Default::default()
        };
        assert_eq!(cells(&read, Some(&command))[2], Value::I64(35_000_000));
        let unregistered = CountryCommandPowerRow { max_command_power: None, ..command };
        assert_eq!(cells(&read, Some(&unregistered))[2], Value::Unrecorded);
    }

    #[test]
    fn its_state_is_its_tables_so_a_resume_recomputes_from_them() {
        let mut stability = Stability::default();
        let mut blob = Bytes::new();
        stability.save(&mut blob).unwrap();
        assert!(blob.bytes().is_empty());
        stability.load(&[]).unwrap();
        assert!(stability.rebuild);
        assert!(stability.load(&[1]).is_err());
    }
}
