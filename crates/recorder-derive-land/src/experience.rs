
use recorder_derive_framework::framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns};
use replay_columnar::leaf::{Reader, Writer as Bytes};
use replay_data::rows::{CountryExperienceGainRow, CountryExperienceRow, TableId};
use std::{
    collections::{HashMap, VecDeque},
    io,
};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

pub const DAY_HOURS: i32 = 24;

const BANKED: [usize; 3] = columns::<CountryExperienceRow, 3>(["army_raw", "navy_raw", "air_raw"]);
const GAINS: [usize; 4] = columns::<CountryExperienceGainRow, 4>(["army_raw", "navy_raw", "air_raw", "complete"]);

const READS: &[Read] = &[
    Read {
        table: TableId::CountryExperience,
        columns: Columns::These(&BANKED),
        watch: Watch::EveryRead,
        required: true,
    },
    Read {
        table: TableId::CountryExperienceGains,
        columns: Columns::These(&GAINS),
        watch: Watch::Changed,
        required: false,
    },
];

type Amounts = [i64; 3];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Country {
    reads: VecDeque<(i32, Amounts)>,
    runs: VecDeque<(i32, Amounts)>,
}

#[derive(Default)]
pub struct Experience {
    countries: HashMap<u32, Country>,
    instances: HashMap<u32, Vec<u32>>,
    stale: bool,
    gains: bool,
    last: Option<i32>,
}

fn trim(runs: &mut VecDeque<(i32, Amounts)>, first: i32) {
    while runs.get(1).is_some_and(|(start, _)| *start <= first) {
        runs.pop_front();
    }
}

fn gained(runs: &mut VecDeque<(i32, Amounts)>, hour: i32) -> Option<[i128; 3]> {
    let first = hour - (DAY_HOURS - 1);
    trim(runs, first);
    if runs.front()?.0 > first {
        return None;
    }
    let mut sum = [0_i128; 3];
    let mut end = i64::from(hour) + 1;
    for &(start, values) in runs.iter().rev() {
        let from = i64::from(start.max(first));
        for (total, value) in sum.iter_mut().zip(values) {
            *total += i128::from(value) * i128::from(end - from);
        }
        end = from;
    }
    Some(sum)
}

fn changed(reads: &VecDeque<(i32, Amounts)>, hour: i32, banked: Amounts) -> Option<[i128; 3]> {
    let (_, before) = reads.iter().find(|(at, _)| *at == hour - DAY_HOURS)?;
    Some(std::array::from_fn(|at| i128::from(banked[at]) - i128::from(before[at])))
}

fn i32_at(reader: &mut Reader<'_>) -> io::Result<i32> {
    Ok(i32::from_le_bytes(reader.raw(4)?.try_into().expect("4 bytes")))
}

fn cell(value: Option<i128>) -> Value {
    value.and_then(|value| i64::try_from(value).ok()).map_or(Value::Unrecorded, Value::I64)
}

fn cells(change: Option<[i128; 3]>, gain: Option<[i128; 3]>) -> Vec<Value> {
    let change = change.map_or([None; 3], |values| values.map(Some));
    let gain = gain.map_or([None; 3], |values| values.map(Some));
    change.into_iter().chain(gain).map(cell).collect()
}

impl Experience {
    fn index(&mut self, hour: &Hour<'_>) {
        self.instances.clear();
        let table = hour.table(TableId::CountryExperienceGains);
        for key in table.rows().filter_map(|slot| table.key(slot)) {
            self.instances.entry((key >> 32) as u32).or_default().push(key as u32);
        }
        self.stale = false;
    }

    fn reindex(&mut self, hour: &Hour<'_>, touched: &mut Vec<u32>) {
        let table = hour.table(TableId::CountryExperienceGains);
        for key in hour.deleted_keys(TableId::CountryExperienceGains) {
            let country = (key >> 32) as u32;
            if let Some(held) = self.instances.get_mut(&country) {
                held.retain(|&instance| instance != key as u32);
                if held.is_empty() {
                    self.instances.remove(&country);
                }
            }
            touched.push(country);
        }
        for &slot in &hour.changes(TableId::CountryExperienceGains).inserted {
            let Some(key) = table.key(slot) else { continue };
            let held = self.instances.entry((key >> 32) as u32).or_default();
            if !held.contains(&(key as u32)) {
                held.push(key as u32);
            }
        }
    }

    fn gain(&self, hour: &Hour<'_>, country: u32) -> Option<Amounts> {
        let [instance] = self.instances.get(&country)?.as_slice() else { return None };
        let table = hour.table(TableId::CountryExperienceGains);
        let slot = table.slot(u128::from(country) << 32 | u128::from(*instance))?;
        if table.number(GAINS[3], slot)? != 1 {
            return None;
        }
        Some([table.number(GAINS[0], slot)?, table.number(GAINS[1], slot)?, table.number(GAINS[2], slot)?])
    }

    fn step(&mut self, hour: &Hour<'_>, country: u32) {
        let value = self.gain(hour, country);
        let held = self.countries.entry(country).or_default();
        match value {
            Some(value) => {
                if held.runs.back().is_none_or(|(_, last)| *last != value) {
                    held.runs.push_back((hour.hour, value));
                }
                trim(&mut held.runs, hour.hour - (DAY_HOURS - 1));
            }
            None => held.runs.clear(),
        }
    }
}

impl Writer for Experience {
    fn name(&self) -> &'static str {
        "Experience"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::CountryExperienceDaily)]
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
        let at = hour.hour;
        let whole = hour.fresh || self.last.is_none_or(|last| last + 1 != at);
        self.last = Some(at);
        if whole {
            self.countries.clear();
        }
        let gains = hour.complete(TableId::CountryExperienceGains);
        if !gains {
            if self.gains {
                self.countries.values_mut().for_each(|country| country.runs.clear());
            }
            self.instances.clear();
        } else if whole || !self.gains || self.stale {
            self.index(hour);
            let countries: Vec<u32> = self.instances.keys().copied().collect();
            for country in countries {
                self.step(hour, country);
            }
        } else {
            let table = hour.table(TableId::CountryExperienceGains);
            let mut touched: Vec<u32> = hour
                .written_rows(TableId::CountryExperienceGains, Columns::These(&GAINS))
                .into_iter()
                .filter_map(|slot| table.key(slot))
                .map(|key| (key >> 32) as u32)
                .collect();
            self.reindex(hour, &mut touched);
            touched.sort_unstable();
            touched.dedup();
            for country in touched {
                self.step(hour, country);
            }
        }
        self.gains = gains;
        self.countries.retain(|_, country| !country.reads.is_empty() || !country.runs.is_empty());
        for key in hour.deleted_keys(TableId::CountryExperience) {
            self.countries.remove(&(key as u32));
            out.delete(TableId::CountryExperienceDaily, key);
        }
        let banked = hour.table(TableId::CountryExperience);
        let read = banked.sampled_at() == Some(at);
        if !read && !whole {
            return Ok(());
        }
        for slot in banked.rows() {
            let Some(key) = banked.key(slot) else { continue };
            let country = self.countries.entry(key as u32).or_default();
            let amounts = BANKED.map(|column| banked.number(column, slot));
            let (change, gain) = match (read, amounts) {
                (true, [Some(army), Some(navy), Some(air)]) => {
                    let amounts = [army, navy, air];
                    country.reads.retain(|(earlier, _)| at - earlier <= DAY_HOURS);
                    let change = changed(&country.reads, at, amounts);
                    country.reads.push_back((at, amounts));
                    (change, gained(&mut country.runs, at))
                }
                (true, _) => (None, gained(&mut country.runs, at)),
                (false, _) => (None, None),
            };
            out.set(TableId::CountryExperienceDaily, key, cells(change, gain));
        }
        self.countries.retain(|_, country| !country.reads.is_empty() || !country.runs.is_empty());
        Ok(())
    }

    fn save(&self, blob: &mut Bytes) -> io::Result<()> {
        match self.last {
            Some(last) => {
                blob.u8(1)?;
                blob.raw(&last.to_le_bytes())?;
            }
            None => blob.u8(0)?,
        }
        blob.u8(u8::from(self.gains))?;
        let mut countries: Vec<(&u32, &Country)> = self.countries.iter().collect();
        countries.sort_unstable_by_key(|(id, _)| **id);
        blob.varint(countries.len() as u64)?;
        for (id, country) in countries {
            blob.varint(u64::from(*id))?;
            for entries in [&country.reads, &country.runs] {
                blob.varint(entries.len() as u64)?;
                for (hour, amounts) in entries {
                    blob.raw(&hour.to_le_bytes())?;
                    for amount in amounts {
                        blob.raw(&amount.to_le_bytes())?;
                    }
                }
            }
        }
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        *self = Self::default();
        let mut reader = Reader::new(blob);
        self.last = match reader.u8()? {
            0 => None,
            _ => Some(i32_at(&mut reader)?),
        };
        self.gains = reader.u8()? != 0;
        let count = reader.count()?;
        if count > blob.len() {
            return Err(invalid("Experience's state counts more countries than it holds"));
        }
        for _ in 0..count {
            let id = reader.varint_u32()?;
            let mut country = Country::default();
            for entries in [&mut country.reads, &mut country.runs] {
                let length = reader.count()?;
                if length > DAY_HOURS as usize + 1 {
                    return Err(invalid("Experience's state holds more than a day of one country"));
                }
                for _ in 0..length {
                    let hour = i32_at(&mut reader)?;
                    let mut amounts = [0; 3];
                    for amount in &mut amounts {
                        *amount = i64::from_le_bytes(reader.raw(8)?.try_into().expect("8 bytes"));
                    }
                    entries.push_back((hour, amounts));
                }
            }
            self.countries.insert(id, country);
        }
        reader.finish()?;
        self.stale = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recorder_derive_framework::{framework::Writers, testing};
    use replay_columnar::{manifest::Status, store::Store, unit::Discard};
    use replay_data::rows::{
        CountryExperienceDailyRow,
        Recorded::{self, NotRecorded, Value as Known},
    };

    fn writers() -> Writers {
        let mut writers = Writers::new(vec![Box::new(Experience::default())]);
        writers.start_run();
        writers
    }

    struct At {
        banked: Option<i64>,
        gain: Option<Option<(i64, bool)>>,
        stamped: bool,
    }

    fn hour(store: &mut Store, writers: &mut Writers, hour: i32, at: &At) {
        let appends: &[&[u8]] = if hour == 0 { &[b"not read"] } else { &[] };
        store.begin_hour(hour, appends).unwrap();
        let banked = TableId::CountryExperience;
        let sampled = store.view(banked.index()).sampled_at();
        match at.banked {
            Some(value) => {
                let (read, sample) = if at.stamped { (true, Some(hour)) } else { (false, sampled) };
                store.status(banked.index(), Status::Complete, read, sample, true).unwrap();
                let row = CountryExperienceRow { country: 7, army_raw: value, navy_raw: 2 * value, air_raw: 3 * value, ..Default::default() };
                testing::put(store, banked, &row, &[]).unwrap();
            }
            None => store.status(banked.index(), Status::Complete, false, sampled, false).unwrap(),
        }
        let gains = TableId::CountryExperienceGains;
        match at.gain {
            Some(gain) => {
                store.status(gains.index(), Status::Complete, true, Some(hour), true).unwrap();
                match gain {
                    Some((value, complete)) => {
                        let row = CountryExperienceGainRow { country: 7, instance: 1, complete: u8::from(complete), army_raw: value, navy_raw: value, air_raw: 0 };
                        testing::put(store, gains, &row, &[]).unwrap();
                    }
                    None => testing::remove(store, gains, 7 << 32 | 1).unwrap(),
                }
            }
            None => store.status(gains.index(), Status::Unavailable { reason: 0 }, true, Some(hour), false).unwrap(),
        }
        store.settle();
        writers.hour(1, hour, hour == 0, store).unwrap();
        store.end_hour(&mut Discard).unwrap();
    }

    fn day(hour: i32) -> At {
        let banked = (hour % DAY_HOURS == 0).then_some(1_000 + 100 * i64::from(hour / DAY_HOURS));
        let gain = match hour {
            30 => Some((20, false)),
            early if early < 10 => Some((10, true)),
            _ => Some((20, true)),
        };
        At { banked, gain: Some(gain), stamped: true }
    }

    fn row(store: &Store) -> CountryExperienceDailyRow {
        let rows = testing::rows(store, TableId::CountryExperienceDaily);
        assert_eq!(rows.len(), 1);
        let view = store.view(TableId::CountryExperienceDaily.index());
        let slot = view.slot(7).unwrap();
        let cell = |column: usize| view.number(column, slot).map_or(Recorded::NotRecorded, Recorded::Value);
        CountryExperienceDailyRow {
            country: 7,
            army_change: cell(0),
            navy_change: cell(1),
            air_change: cell(2),
            army_gain: cell(3),
            navy_gain: cell(4),
            air_gain: cell(5),
        }
    }

    #[test]
    fn a_daily_read_holds_the_change_over_a_day_and_the_days_gains_never_a_guess() {
        let (mut store, mut writers) = (testing::store(), writers());
        hour(&mut store, &mut writers, 0, &day(0));
        let first = row(&store);
        assert_eq!(first.army_change, NotRecorded, "the run's first read has no read a day before");
        assert_eq!(first.army_gain, NotRecorded, "nor a day of gains");
        for at in 1..=24 {
            hour(&mut store, &mut writers, at, &day(at));
            if at == 23 {
                assert_eq!(row(&store), first, "hours between reads write nothing");
            }
        }
        let second = row(&store);
        assert_eq!((second.army_change, second.navy_change, second.air_change), (Known(100), Known(200), Known(300)));
        assert_eq!(second.army_gain, Known(10 * 9 + 20 * 15), "hours 1 to 9 at 10, 10 to 24 at 20");
        assert_eq!(second.air_gain, Known(0), "a recorded zero gain is zero");
        for at in 25..=48 {
            hour(&mut store, &mut writers, at, &day(at));
        }
        let third = row(&store);
        assert_eq!(third.army_change, Known(100), "a read repeating the day before's change");
        assert_eq!(third.army_gain, NotRecorded, "hour 30's interval was not complete");
        for at in 49..=72 {
            hour(&mut store, &mut writers, at, &day(at));
        }
        assert_eq!(row(&store).army_gain, Known(20 * 24), "a full day after the gap counts again");
    }

    #[test]
    fn banked_cells_written_at_an_hour_no_read_stamps_write_no_daily_row() {
        let (mut store, mut writers) = (testing::store(), writers());
        hour(&mut store, &mut writers, 0, &day(0));
        let first = row(&store);
        for at in 1..=24 {
            hour(&mut store, &mut writers, at, &At { stamped: false, ..day(at) });
        }
        assert_eq!(store.view(TableId::CountryExperience.index()).sampled_at(), Some(0));
        assert_eq!(row(&store), first, "the write at hour 24 was no read");
    }

    #[test]
    fn gains_not_complete_restart_the_window_and_a_missing_row_is_no_zero() {
        let (mut store, mut writers) = (testing::store(), writers());
        for at in 0..=48 {
            let mut this = day(at);
            if at == 40 {
                this.gain = None;
            }
            if at == 41 {
                this.gain = Some(None);
            }
            hour(&mut store, &mut writers, at, &this);
        }
        let read = row(&store);
        assert_eq!(read.army_change, Known(100), "banked reads do not depend on gains");
        assert_eq!(read.army_gain, NotRecorded);
    }

    #[test]
    fn a_resume_in_the_middle_writes_what_a_run_never_stopped_writes() {
        let (mut unbroken, mut writing) = (testing::store(), writers());
        for at in 0..=72 {
            hour(&mut unbroken, &mut writing, at, &day(at));
        }
        let (mut stopped, mut before) = (testing::store(), writers());
        for at in 0..=35 {
            hour(&mut stopped, &mut before, at, &day(at));
        }
        let saved = before.save().unwrap();
        let mut resumed = testing::resumed(&stopped, &[b"not read"]);
        let mut after = writers();
        after.load(&saved, &mut resumed).unwrap();
        after.prime(&resumed, 1, 35);
        for at in 36..=72 {
            hour(&mut resumed, &mut after, at, &day(at));
        }
        assert_eq!(testing::rows(&resumed, TableId::CountryExperienceDaily), testing::rows(&unbroken, TableId::CountryExperienceDaily));
        assert_eq!(row(&resumed).army_gain, Known(20 * 24));
    }

    #[test]
    fn its_window_survives_a_save_and_a_load() {
        let mut experience = Experience::default();
        experience.last = Some(40);
        experience.gains = true;
        experience.countries.insert(
            9,
            Country {
                reads: [(24, [1, -2, 3])].into(),
                runs: [(17, [5, 6, i64::MIN]), (30, [0, 0, 0])].into(),
            },
        );
        let mut blob = Bytes::new();
        experience.save(&mut blob).unwrap();
        let mut loaded = Experience::default();
        loaded.load(blob.bytes()).unwrap();
        assert_eq!((loaded.last, loaded.gains, &loaded.countries), (Some(40), true, &experience.countries));
        assert!(loaded.stale);
        assert!(loaded.load(&blob.bytes()[..blob.bytes().len() - 1]).is_err());
    }
}
