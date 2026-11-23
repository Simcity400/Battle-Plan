
use recorder_derive_framework::{
    deployed::{invalid, row},
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns},
};
use engine_fuel::fuel_demand::daily_required;
use replay_columnar::leaf::Writer as Bytes;
use replay_data::rows::{
    CountryFuelRow, FuelDemandHour, FuelDemandPartRow, FuelDemandRead, FuelDemandUnit, Recorded, TableId, assemble_demand_parts,
};
use std::{
    collections::{HashMap, HashSet},
    io,
};

#[cfg(test)]
mod tests;

const DEMAND: [usize; 1] = columns::<CountryFuelRow, 1>(["demand"]);
const PART: [usize; 2] = columns::<FuelDemandPartRow, 2>(["position", "part"]);

const READS: &[Read] = &[
    Read { table: TableId::CountryFuel, columns: Columns::These(&DEMAND), watch: Watch::EveryRead, required: true },
    Read { table: TableId::FuelDemandParts, columns: Columns::Every, watch: Watch::Changed, required: false },
];

fn country_of(key: u128) -> u32 {
    (key >> 83) as u32
}

fn reason(bytes: &[u8], otherwise: &str) -> Vec<u8> {
    if bytes.is_empty() { otherwise.as_bytes().to_vec() } else { bytes.to_vec() }
}

const MISMATCH: &[u8] = b"Current fuel consumption supplying read or native country identity does not match";

fn stale(row: &CountryFuelRow, read: i32) -> bool {
    matches!(&row.demand, Recorded::Value(pack) if pack.hour.at(read) != read)
}

fn pack(row: CountryFuelRow, parts: Result<&[FuelDemandPartRow], &'static str>, read: i32) -> Result<FuelDemandRead, Vec<u8>> {
    let mut rows = [row];
    assemble_demand_parts(&mut rows, parts);
    let [row] = rows;
    let Recorded::Value(pack) = row.demand else {
        return Err(b"Individual current fuel consumption was not recorded".to_vec());
    };
    if pack.hour.at(read) != read {
        return Err(MISMATCH.to_vec());
    }
    if !pack.reason.is_empty() {
        return Err(pack.reason);
    }
    Ok(pack)
}

fn unit(row: &FuelDemandUnit) -> Option<i64> {
    row.rate_raw.map(daily_required)
}

fn cells(pack: Result<FuelDemandRead, Vec<u8>>, text: &mut dyn FnMut(&[u8]) -> Value) -> Vec<Value> {
    let pack = match pack {
        Ok(pack) => pack,
        Err(why) => {
            let mut cells = vec![Value::Unrecorded; 9];
            cells.extend((9..12).map(|_| text(&why)));
            return cells;
        }
    };
    let rates = pack.native_rates();
    let rates = [rates[0], rates[2], rates[1]];
    let causes = [&pack.army_reason, &pack.air_reason, &pack.navy_reason];
    let mut known = [0_i128; 3];
    let mut positive = [0_i128; 3];
    let mut add = |family: usize, daily: i64| {
        known[family] += i128::from(daily);
        positive[family] += i128::from(daily.max(0));
    };
    for daily in pack.divisions.iter().filter_map(unit) {
        add(0, daily);
    }
    for daily in pack.task_forces.iter().filter_map(unit) {
        add(2, daily);
    }
    for wing in pack.sites.iter().flat_map(|site| &site.wings) {
        if wing.reason.is_empty()
            && let Ok(rate) = pack.wing_rate(wing)
        {
            add(1, daily_required(rate));
        }
    }
    let widened = |sum: i128| i64::try_from(sum).map_or(Value::Unrecorded, Value::I64);
    let mut cells: Vec<Value> = rates.iter().map(|rate| rate.map_or(Value::Unrecorded, |rate| Value::I64(daily_required(rate)))).collect();
    cells.extend(known.map(widened));
    cells.extend(positive.map(widened));
    for (rate, cause) in rates.iter().zip(causes) {
        cells.push(match rate {
            Ok(_) => text(b""),
            Err(message) => text(&reason(cause, message)),
        });
    }
    cells
}

#[derive(Default)]
pub struct FuelDemand {
    rebuild: bool,
    parts: HashMap<u32, Vec<u128>>,
    stamped: HashSet<u32>,
}

impl FuelDemand {
    fn parts_of(&self, hour: &Hour<'_>, country: u32) -> io::Result<Vec<FuelDemandPartRow>> {
        let view = hour.table(TableId::FuelDemandParts);
        let mut keys = self.parts.get(&country).cloned().unwrap_or_default();
        keys.sort_unstable();
        keys.into_iter().filter_map(|key| view.slot(key)).map(|slot| row(&view, slot, &PART)).collect()
    }

    fn country(&mut self, hour: &Hour<'_>, country: u32, out: &mut Rows) -> io::Result<()> {
        let view = hour.table(TableId::CountryFuel);
        let Some(slot) = view.slot(u128::from(country)) else {
            self.stamped.remove(&country);
            out.delete(TableId::FuelDemandTotals, u128::from(country));
            return Ok(());
        };
        let fuel: CountryFuelRow = row(&view, slot, &DEMAND)?;
        match &fuel.demand {
            Recorded::Value(read) if read.hour != FuelDemandHour::Host => self.stamped.insert(country),
            _ => self.stamped.remove(&country),
        };
        let read = view.sampled_at().unwrap_or(hour.hour);
        let pack = if stale(&fuel, read) {
            Err(MISMATCH.to_vec())
        } else {
            let parts = if hour.complete(TableId::FuelDemandParts) { Some(self.parts_of(hour, country)?) } else { None };
            pack(fuel, parts.as_deref().ok_or("FuelDemandParts was not complete"), read)
        };
        let cells = cells(pack, &mut |text: &[u8]| out.text(text));
        out.set(TableId::FuelDemandTotals, u128::from(country), cells);
        Ok(())
    }

    fn rebuild(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        *self = Self::default();
        let parts = hour.table(TableId::FuelDemandParts);
        for key in parts.rows().filter_map(|slot| parts.key(slot)) {
            self.parts.entry(country_of(key)).or_default().push(key);
        }
        let fuel = hour.table(TableId::CountryFuel);
        let countries: HashSet<u32> = fuel.rows().filter_map(|slot| fuel.key(slot)).map(|key| key as u32).collect();
        for key in recorder_derive_framework::forces::held(out, TableId::FuelDemandTotals) {
            if !countries.contains(&(key as u32)) {
                out.delete(TableId::FuelDemandTotals, key);
            }
        }
        for country in countries {
            self.country(hour, country, out)?;
        }
        Ok(())
    }

    fn apply(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let mut dirty: HashSet<u32> = hour.deleted_keys(TableId::CountryFuel).map(|key| key as u32).collect();
        let fuel = hour.table(TableId::CountryFuel);
        dirty.extend(hour.written_rows(TableId::CountryFuel, Columns::These(&DEMAND)).into_iter().filter_map(|slot| fuel.key(slot)).map(|key| key as u32));
        if fuel.sampled_at() == Some(hour.hour) {
            dirty.extend(self.stamped.iter().copied());
        }
        if hour.changes(TableId::FuelDemandParts).status_changed {
            dirty.extend(self.parts.keys().copied());
        }
        let deleted: Vec<u128> = hour.deleted_keys(TableId::FuelDemandParts).collect();
        for key in deleted {
            let country = country_of(key);
            if let Some(keys) = self.parts.get_mut(&country) {
                keys.retain(|&held| held != key);
                if keys.is_empty() {
                    self.parts.remove(&country);
                }
            }
            dirty.insert(country);
        }
        let parts = hour.table(TableId::FuelDemandParts);
        let changes = hour.changes(TableId::FuelDemandParts);
        for &slot in &changes.inserted {
            let Some(key) = parts.key(slot) else { continue };
            self.parts.entry(country_of(key)).or_default().push(key);
        }
        for slot in hour.written_rows(TableId::FuelDemandParts, Columns::Every) {
            if let Some(key) = parts.key(slot) {
                dirty.insert(country_of(key));
            }
        }
        for country in dirty {
            self.country(hour, country, out)?;
        }
        Ok(())
    }
}

impl Writer for FuelDemand {
    fn name(&self) -> &'static str {
        "FuelDemand"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::FuelDemandTotals)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        if hour.fresh || self.rebuild { self.rebuild(hour, out) } else { self.apply(hour, out) }
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("FuelDemand's state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}
