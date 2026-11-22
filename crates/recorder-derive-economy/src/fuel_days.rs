
use recorder_derive_framework::{
    deployed::{invalid, number, row},
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns},
};
use engine_fuel::ccountry_fuel_status::HISTORY_RING_ENTRIES;
use replay_columnar::leaf::{Reader, Writer as Bytes};
use replay_data::{
    cadence,
    rows::{CountryFuelRow, CountryLinkRow, CountryRow, FuelAwardDay, FuelDayRow, Recorded, StateRow, TableId},
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io,
};

#[cfg(test)]
mod tests;

const DAY: i32 = HISTORY_RING_ENTRIES as i32;
const LAG: i32 = DAY - 1;

const FUEL: [usize; 7] = columns::<CountryFuelRow, 7>([
    "ring_gain_raw",
    "ring_consumed_raw",
    "ring_other_raw",
    "ring_received_0_raw",
    "ring_received_1_raw",
    "ring_received_2_raw",
    "award_day",
]);
const SLOT: [usize; 1] = columns::<CountryRow, 1>(["canonical_slot"]);
const LINK: [usize; 1] = columns::<CountryLinkRow, 1>(["canonical_slot"]);
const OWNER: [usize; 1] = columns::<StateRow, 1>(["owner"]);

const READS: &[Read] = &[
    Read { table: TableId::CountryFuel, columns: Columns::These(&FUEL), watch: Watch::EveryRead, required: true },
    Read { table: TableId::Countries, columns: Columns::These(&SLOT), watch: Watch::Changed, required: false },
    Read { table: TableId::CountryLinks, columns: Columns::These(&LINK), watch: Watch::Changed, required: false },
    Read { table: TableId::States, columns: Columns::These(&OWNER), watch: Watch::Changed, required: false },
];

const OWNERSHIP: [TableId; 3] = [TableId::Countries, TableId::CountryLinks, TableId::States];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Shared {
    known: bool,
    unresolved: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Presence {
    Absent,
    Unresolved,
    At(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sample {
    Owns(u32),
    Without,
    Unknown,
}

type Log<T> = VecDeque<(i32, T)>;

fn at<T: Copy>(log: &Log<T>, hour: i32) -> Option<T> {
    log.iter().rev().find(|(from, _)| *from <= hour).map(|&(_, value)| value)
}

fn push<T: Copy + PartialEq>(log: &mut Log<T>, hour: i32, value: T) {
    if log.back().is_some_and(|&(_, held)| held == value) {
        return;
    }
    if log.back().is_some_and(|&(from, _)| from == hour) {
        log.pop_back();
        if log.back().is_some_and(|&(_, held)| held == value) {
            return;
        }
    }
    log.push_back((hour, value));
    while log.len() > 1 && log[1].0 <= hour - DAY {
        log.pop_front();
    }
}

enum Day<'a> {
    Ring { gained: i64, burned: i64, received: [i64; 3], other: i64 },
    Awards(&'a FuelAwardDay),
}

fn raw(day: &Day<'_>) -> [Option<i64>; 14] {
    match day {
        Day::Ring { gained, burned, received, other } => {
            [Some(*gained), Some(*burned), Some(received[0]), Some(received[1]), Some(received[2]), Some(*other), None, None, None, None, None, None, None, None]
        }
        Day::Awards(day) => {
            let [base, states, oil, negative] = day.gains_raw;
            let [incoming, outgoing, flat, other] = day.transfers_raw;
            let [army, air, navy] = day.received_raw;
            [day.own_gain_raw, day.consumed_raw, army, air, navy, other, base, states, oil, negative, day.own_gain_quantity_raw, incoming, outgoing, flat].map(Some)
        }
    }
}

fn awards(row: &CountryFuelRow, hour: i32) -> Result<&FuelAwardDay, &[u8]> {
    let Recorded::Value(day) = &row.award_day else {
        return Err(b"Applied fuel source and recipient history not recorded");
    };
    if !day.reason.is_empty() {
        return Err(&day.reason);
    }
    if !day.continuity_start_hour.is_some_and(|start| start <= hour - LAG) {
        return Err(b"Applied fuel observation continuity boundary missing or later than this window");
    }
    if day.last_hour != hour {
        return Err(b"Applied fuel window does not match this supplying read");
    }
    Ok(day)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Kept {
    slot: Option<i32>,
    any: Option<i32>,
    pending: Option<i32>,
}

#[derive(Default)]
pub struct FuelDays {
    observed: Option<i32>,
    shared: Log<Shared>,
    owning: HashMap<u32, Log<bool>>,
    presence: HashMap<u32, Log<Presence>>,
    kept: HashMap<u32, Kept>,
    links: HashMap<u32, u32>,
    countries: HashMap<u32, u32>,
    owners: HashMap<u32, u32>,
    owned: HashMap<u32, u32>,
    unresolved: u32,
    reread: bool,
}

impl FuelDays {
    fn resolve(&self, owner: u32) -> Option<Option<u32>> {
        if owner == 0 {
            return Some(None);
        }
        self.links.get(&owner).map(|&slot| Some(slot))
    }

    fn count(&mut self, owner: u32, by: i32) {
        match self.resolve(owner) {
            Some(None) => {}
            Some(Some(slot)) => {
                let count = self.owned.entry(slot).or_default();
                *count = count.saturating_add_signed(by);
                if *count == 0 {
                    self.owned.remove(&slot);
                }
            }
            None => self.unresolved = self.unresolved.saturating_add_signed(by),
        }
    }

    fn read_tables(&mut self, hour: &Hour<'_>) {
        self.links.clear();
        let links = hour.table(TableId::CountryLinks);
        for slot in links.rows() {
            if let (Some(key), Some(canonical)) = (links.key(slot), number(&links, LINK[0], slot)) {
                self.links.insert(key as u32, canonical as u32);
            }
        }
        self.countries.clear();
        let countries = hour.table(TableId::Countries);
        for slot in countries.rows() {
            if let (Some(key), Some(canonical)) = (countries.key(slot), number(&countries, SLOT[0], slot)) {
                self.countries.insert(key as u32, canonical as u32);
            }
        }
        self.owners.clear();
        let states = hour.table(TableId::States);
        for slot in states.rows() {
            if let (Some(key), Some(owner)) = (states.key(slot), number(&states, OWNER[0], slot)) {
                self.owners.insert(key as u32, owner as u32);
            }
        }
        self.owned.clear();
        self.unresolved = 0;
        let owners: Vec<u32> = self.owners.values().copied().collect();
        for owner in owners {
            self.count(owner, 1);
        }
    }

    fn presence_now(&self, raw: u32) -> Presence {
        match self.countries.get(&raw) {
            None => Presence::Absent,
            Some(&slot) if self.links.get(&raw) == Some(&slot) => Presence::At(slot),
            Some(_) => Presence::Unresolved,
        }
    }

    fn known(hour: &Hour<'_>) -> bool {
        OWNERSHIP.iter().all(|&table| hour.complete(table))
    }

    fn log_all(&mut self, hour: i32, known: bool) {
        push(&mut self.shared, hour, Shared { known, unresolved: self.unresolved > 0 });
        let mut slots: HashSet<u32> = self.owning.keys().copied().collect();
        slots.extend(self.owned.keys().copied());
        for slot in slots {
            let owns = self.owned.contains_key(&slot);
            push(self.owning.entry(slot).or_default(), hour, owns);
        }
        let mut raws: HashSet<u32> = self.presence.keys().copied().collect();
        raws.extend(self.countries.keys().copied());
        for raw in raws {
            let presence = self.presence_now(raw);
            push(self.presence.entry(raw).or_default(), hour, presence);
        }
        self.forget(hour);
    }

    fn forget(&mut self, hour: i32) {
        let old = |log_len: usize, from: i32| log_len == 1 && from <= hour - DAY;
        self.owning.retain(|_, log| !(old(log.len(), log[0].0) && !log[0].1));
        self.presence.retain(|_, log| !(old(log.len(), log[0].0) && log[0].1 == Presence::Absent));
    }

    fn restart(&mut self, hour: &Hour<'_>) {
        let kept = std::mem::take(&mut self.kept);
        *self = Self { kept, ..Self::default() };
        self.read_tables(hour);
        self.observed = Some(hour.hour);
        self.log_all(hour.hour, Self::known(hour));
    }

    fn observe(&mut self, hour: &Hour<'_>) {
        let known = Self::known(hour);
        let whole = OWNERSHIP.iter().any(|&table| hour.changes(table).status_changed)
            || hour.wrote(TableId::CountryLinks, Columns::These(&LINK))
            || hour.wrote(TableId::Countries, Columns::These(&SLOT));
        if whole {
            self.read_tables(hour);
            self.log_all(hour.hour, known);
            return;
        }
        let mut changed: Vec<(u32, Option<u32>)> = hour.deleted_keys(TableId::States).map(|key| (key as u32, None)).collect();
        let states = hour.table(TableId::States);
        for slot in hour.written_rows(TableId::States, Columns::These(&OWNER)) {
            if let (Some(key), Some(owner)) = (states.key(slot), number(&states, OWNER[0], slot)) {
                changed.push((key as u32, Some(owner as u32)));
            }
        }
        if changed.is_empty() {
            return;
        }
        let mut slots = HashSet::new();
        for (state, owner) in changed {
            let before = match owner {
                Some(owner) => self.owners.insert(state, owner),
                None => self.owners.remove(&state),
            };
            for (raw, by) in [(before, -1), (owner, 1)] {
                let Some(raw) = raw else { continue };
                if let Some(Some(slot)) = self.resolve(raw) {
                    slots.insert(slot);
                }
                self.count(raw, by);
            }
        }
        push(&mut self.shared, hour.hour, Shared { known, unresolved: self.unresolved > 0 });
        for slot in slots {
            let owns = self.owned.contains_key(&slot);
            push(self.owning.entry(slot).or_default(), hour.hour, owns);
        }
        self.forget(hour.hour);
    }

    fn sample(&self, raw: u32, hour: i32) -> Sample {
        if self.observed.is_none_or(|first| hour < first) {
            return Sample::Unknown;
        }
        let Some(shared) = at(&self.shared, hour).filter(|shared| shared.known) else {
            return Sample::Unknown;
        };
        match self.presence.get(&raw).and_then(|log| at(log, hour)).unwrap_or(Presence::Absent) {
            Presence::Absent => Sample::Without,
            Presence::Unresolved => Sample::Unknown,
            Presence::At(slot) if self.owning.get(&slot).and_then(|log| at(log, hour)).unwrap_or(false) => Sample::Owns(slot),
            Presence::At(_) if shared.unresolved => Sample::Unknown,
            Presence::At(_) => Sample::Without,
        }
    }

    fn left_out(&self, raw: u32, read: i32) -> Option<u8> {
        let identity = match self.sample(raw, read) {
            Sample::Owns(slot) => Some(slot),
            _ => match self.presence.get(&raw).and_then(|log| at(log, read)) {
                Some(Presence::At(slot)) if at(&self.shared, read).is_some_and(|shared| shared.known) => Some(slot),
                _ => None,
            },
        };
        let mut unknown = false;
        for hour in read - DAY..read {
            match self.sample(raw, hour) {
                Sample::Owns(slot) if identity == Some(slot) => {}
                Sample::Owns(_) if identity.is_none() => unknown = true,
                Sample::Unknown => unknown = true,
                Sample::Owns(_) | Sample::Without => return Some(FuelDayRow::NOT_UPDATED),
            }
        }
        unknown.then_some(FuelDayRow::OWNERSHIP_NOT_RECORDED)
    }

    fn day(&mut self, row: &CountryFuelRow, read: i32, out: &mut Rows) -> Vec<Value> {
        let applied = awards(row, read);
        let mut left_out = match applied {
            Ok(_) => None,
            Err(_) => self.left_out(row.country, read),
        };
        let mut day = left_out.is_none().then(|| match applied {
            Ok(day) => Day::Awards(day),
            Err(_) => Day::Ring {
                gained: row.ring_gain_raw,
                burned: row.ring_consumed_raw,
                received: [row.ring_received_0_raw, row.ring_received_1_raw, row.ring_received_2_raw],
                other: row.ring_other_raw,
            },
        });
        let mut overlapped = None;
        if day.is_some() {
            let kept = self.kept.entry(row.country).or_default();
            let near = |kept: Option<i32>| kept.is_some_and(|kept| read - kept <= LAG);
            let blocked = if cadence::is_refresh(TableId::CountryFuel, read) { near(kept.slot) } else { near(kept.any) };
            if blocked {
                left_out = Some(FuelDayRow::OVERLAPS);
                day = None;
            } else if cadence::is_refresh(TableId::CountryFuel, read) {
                kept.slot = Some(read);
                kept.any = Some(read);
                overlapped = kept.pending.take().filter(|&pending| read - pending <= LAG);
            } else {
                kept.any = Some(read);
                kept.pending = Some(read);
            }
        }
        let mut cells = vec![Value::I32(read)];
        cells.extend(day.as_ref().map_or([None; 14], raw).map(|value| value.map_or(Value::Unrecorded, Value::I64)));
        cells.push(Value::U8(left_out.unwrap_or(FuelDayRow::KEPT)));
        cells.push(out.text(applied.err().unwrap_or(b"")));
        let continuity = match &row.award_day {
            Recorded::Value(day) => day.continuity_start_hour,
            Recorded::NotRecorded => None,
        };
        cells.push(Value::OptionalI32(continuity));
        cells.push(Value::OptionalI32(overlapped));
        cells
    }

    fn read(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let view = hour.table(TableId::CountryFuel);
        let mut live = HashSet::new();
        for slot in view.rows() {
            let fuel: CountryFuelRow = row(&view, slot, &FUEL)?;
            live.insert(fuel.country);
            let cells = self.day(&fuel, hour.hour, out);
            out.set(TableId::FuelDays, u128::from(fuel.country), cells);
        }
        self.kept.retain(|country, _| live.contains(country));
        for key in recorder_derive_framework::forces::held(out, TableId::FuelDays) {
            if !live.contains(&(key as u32)) {
                out.delete(TableId::FuelDays, key);
            }
        }
        Ok(())
    }
}

impl Writer for FuelDays {
    fn name(&self) -> &'static str {
        "FuelDays"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::FuelDays)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        if hour.fresh {
            self.restart(hour);
        } else {
            if std::mem::take(&mut self.reread) {
                self.read_tables(hour);
            }
            self.observe(hour);
        }
        if hour.table(TableId::CountryFuel).sampled_at() == Some(hour.hour) {
            self.read(hour, out)?;
        } else {
            let gone: Vec<u128> = hour.deleted_keys(TableId::CountryFuel).collect();
            for key in gone {
                self.kept.remove(&(key as u32));
                out.delete(TableId::FuelDays, key);
            }
        }
        Ok(())
    }

    fn save(&self, blob: &mut Bytes) -> io::Result<()> {
        fn hour(blob: &mut Bytes, hour: Option<i32>) -> io::Result<()> {
            match hour {
                None => blob.u8(0),
                Some(hour) => {
                    blob.u8(1)?;
                    blob.varint(u64::from(hour as u32))
                }
            }
        }
        hour(blob, self.observed)?;
        blob.varint(self.shared.len() as u64)?;
        for &(from, shared) in &self.shared {
            blob.varint(u64::from(from as u32))?;
            blob.u8(u8::from(shared.known) | u8::from(shared.unresolved) << 1)?;
        }
        let mut owning: Vec<(&u32, &Log<bool>)> = self.owning.iter().collect();
        owning.sort_unstable_by_key(|(slot, _)| **slot);
        blob.varint(owning.len() as u64)?;
        for (slot, log) in owning {
            blob.varint(u64::from(*slot))?;
            blob.varint(log.len() as u64)?;
            for &(from, owns) in log {
                blob.varint(u64::from(from as u32))?;
                blob.u8(u8::from(owns))?;
            }
        }
        let mut presence: Vec<(&u32, &Log<Presence>)> = self.presence.iter().collect();
        presence.sort_unstable_by_key(|(raw, _)| **raw);
        blob.varint(presence.len() as u64)?;
        for (raw, log) in presence {
            blob.varint(u64::from(*raw))?;
            blob.varint(log.len() as u64)?;
            for &(from, place) in log {
                blob.varint(u64::from(from as u32))?;
                match place {
                    Presence::Absent => blob.u8(0)?,
                    Presence::Unresolved => blob.u8(1)?,
                    Presence::At(slot) => {
                        blob.u8(2)?;
                        blob.varint(u64::from(slot))?;
                    }
                }
            }
        }
        let mut kept: Vec<(&u32, &Kept)> = self.kept.iter().collect();
        kept.sort_unstable_by_key(|(raw, _)| **raw);
        blob.varint(kept.len() as u64)?;
        for (raw, kept) in kept {
            blob.varint(u64::from(*raw))?;
            hour(blob, kept.slot)?;
            hour(blob, kept.any)?;
            hour(blob, kept.pending)?;
        }
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        fn hour(reader: &mut Reader<'_>) -> io::Result<Option<i32>> {
            Ok(match reader.u8()? {
                0 => None,
                _ => Some(reader.varint_u32()? as i32),
            })
        }
        fn count(reader: &mut Reader<'_>, blob: &[u8]) -> io::Result<usize> {
            let count = reader.count()?;
            if count > blob.len() {
                return Err(invalid("FuelDays' state counts more entries than it holds"));
            }
            Ok(count)
        }
        *self = Self { reread: true, ..Self::default() };
        let mut reader = Reader::new(blob);
        self.observed = hour(&mut reader)?;
        for _ in 0..count(&mut reader, blob)? {
            let from = reader.varint_u32()? as i32;
            let flags = reader.u8()?;
            self.shared.push_back((from, Shared { known: flags & 1 != 0, unresolved: flags & 2 != 0 }));
        }
        for _ in 0..count(&mut reader, blob)? {
            let slot = reader.varint_u32()?;
            let log = self.owning.entry(slot).or_default();
            for _ in 0..count(&mut reader, blob)? {
                let from = reader.varint_u32()? as i32;
                log.push_back((from, reader.u8()? != 0));
            }
        }
        for _ in 0..count(&mut reader, blob)? {
            let raw = reader.varint_u32()?;
            let mut log = Log::new();
            for _ in 0..count(&mut reader, blob)? {
                let from = reader.varint_u32()? as i32;
                let place = match reader.u8()? {
                    0 => Presence::Absent,
                    1 => Presence::Unresolved,
                    _ => Presence::At(reader.varint_u32()?),
                };
                log.push_back((from, place));
            }
            self.presence.insert(raw, log);
        }
        for _ in 0..count(&mut reader, blob)? {
            let raw = reader.varint_u32()?;
            let kept = Kept { slot: hour(&mut reader)?, any: hour(&mut reader)?, pending: hour(&mut reader)? };
            self.kept.insert(raw, kept);
        }
        reader.finish()
    }

    fn prime(&mut self, hour: &Hour<'_>) {
        self.read_tables(hour);
        self.reread = false;
    }
}
