
use recorder_derive_framework::framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns};
use replay_columnar::{
    leaf::Writer as Bytes,
    store::{TableView, Value as Held},
};
use replay_data::rows::{TableId, TagHolderRow, ThreatEventRow, ThreatSourceRow};
use std::{collections::HashMap, io};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

const WATCHED: [usize; 5] = columns::<ThreatSourceRow, 5>(["country", "second_country", "date", "reason", "final_raw"]);
const LINK: [usize; 1] = [0];

const READS: &[Read] = &[
    Read {
        table: TableId::ThreatSources,
        columns: Columns::These(&WATCHED),
        watch: Watch::Changed,
        required: true,
    },
    Read {
        table: TableId::CountryLinks,
        columns: Columns::These(&LINK),
        watch: Watch::Changed,
        required: false,
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
struct Source {
    ordinal: u32,
    country: u32,
    second: u32,
    date: i32,
    reason: Box<[u8]>,
    place: u32,
    final_raw: i64,
    slot: u32,
}

type Identity<'a> = (u32, u32, i32, &'a [u8], u32);

impl Source {
    fn identity(&self) -> Identity<'_> {
        (self.country, self.second, self.date, &self.reason, self.place)
    }
}

#[derive(Default)]
pub struct Threats {
    listing: Vec<Source>,
    living: Vec<u128>,
    last: Option<i32>,
}

fn number(table: &TableView<'_>, column: usize, slot: u32) -> Option<i64> {
    match table.value(column, slot) {
        Held::Number(bits) => Some(bits as i64),
        _ => None,
    }
}

fn resolve(hour: &Hour<'_>, country: u32) -> Option<u32> {
    if !hour.complete(TableId::CountryLinks) {
        return None;
    }
    let links = hour.table(TableId::CountryLinks);
    links.slot(u128::from(country)).and_then(|slot| number(&links, LINK[0], slot)).map(|canonical| canonical as u32)
}

fn listing(hour: &Hour<'_>) -> io::Result<Vec<Source>> {
    let table = hour.table(TableId::ThreatSources);
    let mut rows: Vec<(u32, u32)> = table.rows().filter_map(|slot| Some((table.key(slot)? as u32, slot))).collect();
    rows.sort_unstable();
    let mut places: HashMap<(u32, u32, i32, Box<[u8]>), u32> = HashMap::new();
    let mut listed = Vec::with_capacity(rows.len());
    for (ordinal, slot) in rows {
        let unread = || invalid(format!("ThreatSources row {ordinal} does not hold every cell it is known by"));
        let [country, second, date, final_raw] = [WATCHED[0], WATCHED[1], WATCHED[2], WATCHED[4]].map(|column| number(&table, column, slot));
        let (Some(country), Some(second), Some(date), Some(final_raw)) = (country, second, date, final_raw) else { return Err(unread()) };
        let reason: Box<[u8]> = table.text(WATCHED[3], slot).and_then(|id| hour.text(id)).ok_or_else(unread)?.into();
        let (country, second, date) = (country as u32, second as u32, date as i32);
        let place = places.entry((country, second, date, reason.clone())).or_default();
        listed.push(Source {
            ordinal,
            country,
            second,
            date,
            reason,
            place: *place,
            final_raw,
            slot: resolve(hour, country).unwrap_or(TagHolderRow::NONE),
        });
        *place += 1;
    }
    Ok(listed)
}

impl Threats {
    fn write(&mut self, out: &mut Rows, kind: u8, hour: i32, source: &Source, slot: u32) {
        let key = u128::from(kind) << 96 | u128::from(slot) << 64 | u128::from(hour as u32) << 32 | u128::from(source.ordinal);
        let reason = out.text(&source.reason);
        let cells = vec![Value::U32(source.country), Value::U32(source.second), Value::I32(source.date), reason, Value::I64(source.final_raw), Value::U32(source.place)];
        out.set(TableId::ThreatEvents, key, cells);
        self.living.push(key);
    }
}

impl Writer for Threats {
    fn name(&self) -> &'static str {
        "Threats"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::ThreatEvents)]
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
            out.delete(TableId::ThreatEvents, key);
        }
        let at = hour.hour;
        let fresh = hour.fresh || self.last.is_none_or(|last| last + 1 != at);
        self.last = Some(at);
        if fresh {
            self.listing = listing(hour)?;
            return Ok(());
        }
        if !hour.wrote(TableId::ThreatSources, Columns::These(&WATCHED)) {
            return Ok(());
        }
        let now = listing(hour)?;
        let before = std::mem::take(&mut self.listing);
        let mut held: HashMap<Identity<'_>, &Source> = before.iter().map(|source| (source.identity(), source)).collect();
        for source in &now {
            match held.remove(&source.identity()) {
                None => self.write(out, ThreatEventRow::LISTED, at, source, source.slot),
                Some(earlier) if earlier.final_raw != source.final_raw => self.write(out, ThreatEventRow::CHANGED, at, source, source.slot),
                Some(_) => {}
            }
        }
        let mut dropped: Vec<&Source> = held.into_values().collect();
        dropped.sort_unstable_by_key(|source| source.ordinal);
        for source in dropped {
            let slot = resolve(hour, source.country).unwrap_or(source.slot);
            self.write(out, ThreatEventRow::DROPPED, at, source, slot);
        }
        self.listing = now;
        Ok(())
    }

    fn prime(&mut self, hour: &Hour<'_>) {
        let events = hour.table(TableId::ThreatEvents);
        self.living = events.rows().filter_map(|slot| events.key(slot)).collect();
        match listing(hour) {
            Ok(listed) => {
                self.listing = listed;
                self.last = Some(hour.hour);
            }
            Err(_) => {
                self.listing.clear();
                self.last = None;
            }
        }
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("Threats' state holds nothing"));
        }
        *self = Self::default();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use recorder_derive_framework::{framework::Writers, testing};
    use replay_columnar::{manifest::Status, store::Store, unit::Discard};
    use replay_data::rows::CountryLinkRow;

    const TEXTS: &[&[u8]] = &[b"annexed", b"justified"];

    fn writers() -> Writers {
        let mut writers = Writers::new(vec![Box::new(Threats::default())]);
        writers.start_run();
        writers
    }

    fn source(country: u32, date: i32, reason: usize, added: i64, current: i64) -> ThreatSourceRow {
        ThreatSourceRow { country, date, reason: TEXTS[reason].to_vec(), final_raw: added, current_raw: current, ..Default::default() }
    }

    fn listing_at(hour: i32) -> Vec<ThreatSourceRow> {
        let a = |added| source(1, 100, 0, added, hour.into());
        let b = source(2, 200, 1, 5, hour.into());
        let c = source(1, 300, 1, 7, hour.into());
        match hour {
            0 => vec![a(50), b.clone()],
            1 => vec![a(60), b.clone(), c.clone()],
            2 | 3 => vec![b.clone(), c.clone()],
            4 => vec![b.clone(), c.clone(), a(60)],
            _ => vec![b.clone(), c.clone(), a(60), b.clone()],
        }
    }

    fn run(store: &mut Store, writers: &mut Writers, hour: i32) {
        store.begin_hour(hour, if hour == 0 { TEXTS } else { &[] }).unwrap();
        for table in [TableId::ThreatSources, TableId::CountryLinks] {
            store.status(table.index(), Status::Complete, true, Some(hour), true).unwrap();
        }
        if hour == 0 {
            testing::put(store, TableId::CountryLinks, &CountryLinkRow { id: 1, canonical_slot: 10 }, TEXTS).unwrap();
        }
        let listed = listing_at(hour);
        let held: Vec<u128> = testing::rows(store, TableId::ThreatSources).into_iter().map(|(key, _)| key).collect();
        for key in held.into_iter().filter(|&key| key as usize >= listed.len()) {
            testing::remove(store, TableId::ThreatSources, key).unwrap();
        }
        for (ordinal, row) in listed.into_iter().enumerate() {
            testing::put(store, TableId::ThreatSources, &ThreatSourceRow { ordinal: ordinal as u32, ..row }, TEXTS).unwrap();
        }
        store.settle();
        writers.hour(1, hour, hour == 0, store).unwrap();
        store.end_hour(&mut Discard).unwrap();
    }

    fn events(store: &Store) -> Vec<(u8, u32, u32, u32, u32)> {
        let view = store.view(TableId::ThreatEvents.index());
        testing::rows(store, TableId::ThreatEvents)
            .into_iter()
            .map(|(key, _)| {
                let slot = view.slot(key).unwrap();
                let [country, place] = [0, 5].map(|column| view.number(column, slot).unwrap() as u32);
                ((key >> 96) as u8, (key >> 64) as u32, key as u32, country, place)
            })
            .collect()
    }

    const NONE: u32 = TagHolderRow::NONE;

    #[test]
    fn a_source_listed_changed_dropped_and_listed_again_is_a_row_each_and_moving_up_is_none() {
        let (mut store, mut writers) = (testing::store(), writers());
        let mut seen = Vec::new();
        for at in 0..=5 {
            run(&mut store, &mut writers, at);
            seen.push(events(&store));
        }
        let (listed, changed, dropped) = (ThreatEventRow::LISTED, ThreatEventRow::CHANGED, ThreatEventRow::DROPPED);
        assert_eq!(
            seen,
            [
                vec![],
                vec![(listed, 10, 2, 1, 0), (changed, 10, 0, 1, 0)],
                vec![(dropped, 10, 0, 1, 0)],
                vec![],
                vec![(listed, 10, 2, 1, 0)],
                vec![(listed, NONE, 3, 2, 1)],
            ],
            "the run's first hour is a baseline; b and c moving up are no events; a second b equal in all four is a source of its own, the second in place"
        );
    }

    #[test]
    fn a_resume_in_the_middle_writes_what_a_run_never_stopped_writes() {
        let (mut unbroken, mut writing) = (testing::store(), writers());
        let mut expected = Vec::new();
        for at in 0..=5 {
            run(&mut unbroken, &mut writing, at);
            expected.push(testing::rows(&unbroken, TableId::ThreatEvents));
        }
        let (mut stopped, mut before) = (testing::store(), writers());
        for at in 0..=1 {
            run(&mut stopped, &mut before, at);
        }
        let saved = before.save().unwrap();
        let mut resumed = testing::resumed(&stopped, TEXTS);
        let mut after = writers();
        after.load(&saved, &mut resumed).unwrap();
        after.prime(&resumed, 1, 1);
        for at in 2..=5 {
            run(&mut resumed, &mut after, at);
            assert_eq!(testing::rows(&resumed, TableId::ThreatEvents), expected[at as usize], "hour {at}");
        }
    }
}
