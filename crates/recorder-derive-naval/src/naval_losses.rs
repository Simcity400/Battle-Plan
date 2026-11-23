
use recorder_derive_framework::framework::{
    Columns, Definition, Hour, Output, Read, Rows, Uses, Value, Watch, Writer,
    definitions::{Define, Form},
};
use replay_columnar::{
    leaf::{Reader, Writer as Bytes},
    manifest::Status,
};
use replay_data::rows::{NavalLossTotalRow as Total, SunkConvoyRow, SunkShipEntry, TableId};
use std::{collections::HashMap, io};

mod inputs;
#[cfg(test)]
mod tests;

use inputs::*;

const READS: &[Read] = &[
    Read { table: TableId::Countries, columns: Columns::These(&COUNTRY_SLOT), watch: Watch::Changed, required: true },
    Read { table: TableId::CountryLinks, columns: Columns::These(&LINK_SLOT), watch: Watch::Changed, required: true },
    Read { table: TableId::SunkShips, columns: Columns::Every, watch: Watch::EveryRead, required: false },
    Read { table: TableId::SunkConvoys, columns: Columns::These(&CONVOY_COUNT), watch: Watch::EveryRead, required: false },
];

const SHIP_RETENTION: Define = Define { section: "NNavy", name: "NAVAL_COMBAT_RESULT_TIMEOUT_YEARS", form: Form::Int32 };
const CONVOY_RETENTION: Define = Define { section: "NNavy", name: "CONVOY_LOSS_HISTORY_TIMEOUT_MONTHS", form: Form::Int32 };

const USES: &[Uses] = &[Uses { definition: Definition::Define(SHIP_RETENTION), required: false }, Uses { definition: Definition::Define(CONVOY_RETENTION), required: false }];

const YEAR: i64 = 365 * 24;

fn key(slot: u32, country: u32, attribution: u8, kind: u8) -> u128 {
    u128::from(slot) << 48 | u128::from(country) << 16 | u128::from(attribution) << 8 | u128::from(kind)
}

fn run_key(kind: u8) -> u128 {
    key(Total::RUN, Total::RUN, Total::LOST, kind)
}

fn signature(entry: &SunkShipEntry, seed: u64) -> u64 {
    let mut hash = seed;
    let mut eat = |bytes: &[u8]| {
        for &byte in bytes {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01B3);
        }
    };
    for text in [&entry.ship_name, &entry.attribution_name, &entry.class_name, &entry.attribution_class_name] {
        eat(&(text.len() as u64).to_le_bytes());
        eat(text);
    }
    for word in [entry.victim_country, entry.attacker_country, entry.sink_hour as u32, entry.class_token, entry.attribution_class_token, entry.model_level as u32, entry.province] {
        eat(&word.to_le_bytes());
    }
    for word in [entry.victim_equipment_variant, entry.damage_source_reference, entry.battle] {
        eat(&word.to_le_bytes());
    }
    eat(&[entry.attribution_class_present, entry.province_present, entry.cause_record, entry.assisted_kill]);
    let mut mixed = hash ^ seed.rotate_left(17);
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

const SIGNATURE_SEED: u64 = 0xCBF2_9CE4_8422_2325;
const CHECK_SEED: u64 = 0x6C62_272E_07BB_0142;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Kind {
    counted_from: Option<i32>,
    lost_from: Option<i32>,
    known: bool,
    unread: Option<(i32, i32)>,
}

impl Kind {
    fn unread(&mut self, first: i32, last: i32) {
        if first <= last {
            self.unread = Some(self.unread.map_or((first, last), |(held, _)| (held, last)));
        }
    }

    fn cells(&self) -> Vec<Value> {
        vec![Value::Unrecorded, Value::Unrecorded, self.counted_from.map_or(Value::Unrecorded, Value::I32), Value::Bool(self.known), Value::OptionalI32(self.lost_from)]
    }
}

#[derive(Default)]
pub struct NavalLosses {
    links: HashMap<u32, u32>,
    identities: HashMap<u32, Vec<u32>>,
    ships: HashMap<u64, (u32, u64)>,
    convoys: HashMap<u128, i32>,
    kinds: [Kind; 2],
    last: Option<i32>,
}

type Pending = HashMap<u128, (i64, u32)>;

impl NavalLosses {
    fn read_identities(&mut self, hour: &Hour<'_>) {
        self.links.clear();
        let links = hour.table(TableId::CountryLinks);
        self.links.extend(links.rows().filter_map(|slot| slotted(&links, slot, LINK_SLOT[0])));
        self.identities.clear();
        let countries = hour.table(TableId::Countries);
        for (raw, slot) in countries.rows().filter_map(|slot| slotted(&countries, slot, COUNTRY_SLOT[0])) {
            if self.links.get(&raw) == Some(&slot) {
                self.identities.entry(slot).or_default().push(raw);
            }
        }
        self.identities.values_mut().for_each(|ids| ids.sort_unstable());
    }

    fn targets(&self, raw: u32, attribution: u8, kind: u8) -> Vec<u128> {
        match self.links.get(&raw) {
            None => vec![key(Total::NONE, raw, attribution, kind)],
            Some(&slot) => match self.identities.get(&slot) {
                Some(ids) => ids.iter().map(|&id| key(slot, id, attribution, kind)).collect(),
                None => vec![key(slot, Total::NONE, attribution, kind)],
            },
        }
    }

    fn add(&self, pending: &mut Pending, raw: u32, attribution: u8, kind: u8, amount: i64, refused: bool) {
        for target in self.targets(raw, attribution, kind) {
            let held = pending.entry(target).or_default();
            if refused {
                held.1 = held.1.saturating_add(amount as u32);
            } else {
                held.0 = held.0.saturating_add(amount);
            }
        }
    }

    fn state(&mut self, hour: &Hour<'_>, table: TableId, kind: u8) -> (bool, Option<i32>) {
        let view = hour.table(table);
        let held = &mut self.kinds[usize::from(kind)];
        held.known = view.status() == Status::Complete;
        if !held.known {
            held.unread(hour.hour, hour.hour);
            return (false, None);
        }
        let read = view.sampled_at() == Some(hour.hour);
        (read, view.sampled_at())
    }

    fn retained(&mut self, hour: &Hour<'_>, kind: u8) {
        let Some((first, last)) = self.kinds[usize::from(kind)].unread.take() else { return };
        let (define, ships) = if kind == Total::SHIPS { (SHIP_RETENTION, true) } else { (CONVOY_RETENTION, false) };
        let window = hour.definitions().define(define.section, define.name).ok().filter(|&value| value > 0);
        let lost = match window {
            None => true,
            Some(years) if ships => i64::from(last) - i64::from(first) + 1 >= years.saturating_mul(YEAR),
            Some(months) => match (SunkConvoyRow::month_of(first), last.checked_add(1).and_then(SunkConvoyRow::month_of)) {
                (Some(first), Some(resume)) => i64::from(resume) - months - 1 >= i64::from(first),
                _ => true,
            },
        };
        let held = &mut self.kinds[usize::from(kind)];
        if lost && held.lost_from.is_none() {
            held.lost_from = Some(hour.hour);
        }
    }

    fn ships(&mut self, hour: &Hour<'_>, pending: &mut Pending) -> io::Result<()> {
        let (read, sampled) = self.state(hour, TableId::SunkShips, Total::SHIPS);
        if !self.kinds[0].known {
            return Ok(());
        }
        let baseline = self.kinds[0].counted_from.is_none();
        let compare = read && (hour.changed(TableId::SunkShips) || self.kinds[0].unread.is_some());
        if !baseline && !compare {
            if read {
                self.retained(hour, Total::SHIPS);
            }
            return Ok(());
        }
        let entries = sunk_ships(hour)?;
        let mut now: HashMap<u64, (u32, u64, usize)> = HashMap::with_capacity(entries.len());
        for (at, entry) in entries.iter().enumerate() {
            let check = signature(entry, CHECK_SEED);
            let held = now.entry(signature(entry, SIGNATURE_SEED)).or_insert((0, check, at));
            if held.1 != check {
                return Err(invalid("Two sunk ships' signatures share a hash"));
            }
            held.0 += 1;
        }
        if baseline {
            self.kinds[0].counted_from = Some(sampled.unwrap_or(hour.hour));
            self.kinds[0].unread = None;
        } else {
            let from = self.kinds[0].counted_from.unwrap_or(hour.hour);
            for (signature, &(count, check, at)) in &now {
                let before = match self.ships.get(signature) {
                    Some(&(_, held_check)) if held_check != check => return Err(invalid("Two sunk ships' signatures share a hash")),
                    Some(&(held, _)) => held,
                    None => 0,
                };
                if count <= before {
                    continue;
                }
                let entry = &entries[at];
                if entry.sink_hour < from {
                    continue;
                }
                let rise = i64::from(count - before);
                let refused = entry.sink_hour > hour.hour || entry.cause_record > 1 || entry.assisted_kill > 1 || entry.province_present > 1 || entry.attribution_class_present > 1;
                self.add(pending, entry.victim_country, Total::LOST, Total::SHIPS, rise, refused);
                if refused || (entry.cause_record == 0 && entry.assisted_kill == 0) {
                    self.add(pending, entry.attacker_country, Total::SUNK_BY, Total::SHIPS, rise, refused);
                }
            }
            self.retained(hour, Total::SHIPS);
        }
        self.ships = now.into_iter().map(|(signature, (count, check, _))| (signature, (count, check))).collect();
        Ok(())
    }

    fn convoys(&mut self, hour: &Hour<'_>, pending: &mut Pending) -> io::Result<()> {
        let table = TableId::SunkConvoys;
        let (read, sampled) = self.state(hour, table, Total::CONVOYS);
        if !self.kinds[1].known {
            return Ok(());
        }
        let view = hour.table(table);
        if self.kinds[1].counted_from.is_none() {
            self.convoys.clear();
            for slot in view.rows() {
                let (key, _, _, count) = convoy(&view, slot)?;
                self.convoys.insert(key, count);
            }
            self.kinds[1].counted_from = Some(sampled.unwrap_or(hour.hour));
            self.kinds[1].unread = None;
            return Ok(());
        }
        if !read {
            return Ok(());
        }
        let visit: Vec<u32> = if self.kinds[1].unread.is_some() {
            let live: std::collections::HashSet<u128> = view.rows().filter_map(|slot| view.key(slot)).collect();
            self.convoys.retain(|key, _| live.contains(key));
            view.rows().collect()
        } else {
            for key in hour.deleted_keys(table) {
                if view.slot(key).is_none() {
                    self.convoys.remove(&key);
                }
            }
            hour.written_rows(table, Columns::These(&CONVOY_COUNT))
        };
        for slot in visit {
            let (key, killer, owner, count) = convoy(&view, slot)?;
            let last = self.convoys.insert(key, count).unwrap_or(0);
            let change = i64::from(count) - i64::from(last);
            if change != 0 {
                self.add(pending, owner, Total::LOST, Total::CONVOYS, change, false);
                self.add(pending, killer, Total::SUNK_BY, Total::CONVOYS, change, false);
            }
        }
        self.retained(hour, Total::CONVOYS);
        Ok(())
    }

    fn read_kinds(&mut self, hour: &Hour<'_>) {
        let view = hour.table(TableId::NavalLossTotals);
        for kind in [Total::SHIPS, Total::CONVOYS] {
            let held = &mut self.kinds[usize::from(kind)];
            let Some(slot) = view.slot(run_key(kind)) else {
                *held = Kind { unread: held.unread, ..Kind::default() };
                continue;
            };
            held.counted_from = view.number(2, slot).map(|hour| hour as i32);
            held.known = view.number(3, slot) == Some(1);
            held.lost_from = view.number(4, slot).map(|hour| hour as i32);
        }
    }
}

impl Writer for NavalLosses {
    fn name(&self) -> &'static str {
        "NavalLosses"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::NavalLossTotals)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn hourly(&self) -> bool {
        true
    }

    fn definitions(&self) -> &'static [Uses] {
        USES
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let at = hour.hour;
        let restart = hour.first || (hour.fresh && self.last.is_none());
        let identities = hour.fresh || hour.wrote(TableId::Countries, Columns::These(&COUNTRY_SLOT)) || hour.wrote(TableId::CountryLinks, Columns::These(&LINK_SLOT));
        let moved = |table: TableId| {
            let view = hour.table(table);
            hour.changes(table).status_changed || view.status() != Status::Complete || view.sampled_at() == Some(at)
        };
        if !identities && !moved(TableId::SunkShips) && !moved(TableId::SunkConvoys) {
            self.last = Some(at);
            return Ok(());
        }
        if restart {
            if !hour.first {
                for key in recorder_derive_framework::forces::held(out, TableId::NavalLossTotals) {
                    out.delete(TableId::NavalLossTotals, key);
                }
            }
            *self = Self::default();
        } else if hour.fresh
            && let Some(last) = self.last
        {
            for kind in &mut self.kinds {
                kind.unread(last + 1, at - 1);
            }
        }
        if identities {
            self.read_identities(hour);
        }
        let mut pending = Pending::new();
        self.ships(hour, &mut pending)?;
        self.convoys(hour, &mut pending)?;
        for (target, (count, refused)) in pending {
            let totals = out.table(TableId::NavalLossTotals);
            let held = totals.slot(target);
            let total = match held {
                Some(slot) => totals.number(0, slot).and_then(|total| total.checked_add(count)),
                None => Some(count),
            };
            let unknown = held.map_or(Some(0), |slot| totals.number(1, slot)).map(|unknown| (unknown as u32).saturating_add(refused));
            out.set(TableId::NavalLossTotals, target, vec![total.map_or(Value::Unrecorded, Value::I64), unknown.map_or(Value::Unrecorded, Value::U32), Value::Unrecorded, Value::Unrecorded, Value::Unrecorded]);
        }
        for kind in [Total::SHIPS, Total::CONVOYS] {
            out.set(TableId::NavalLossTotals, run_key(kind), self.kinds[usize::from(kind)].cells());
        }
        self.last = Some(at);
        Ok(())
    }

    fn prime(&mut self, hour: &Hour<'_>) {
        self.read_identities(hour);
        self.read_kinds(hour);
        self.ships.clear();
        if self.kinds[0].counted_from.is_some()
            && let Ok(entries) = sunk_ships(hour)
        {
            for entry in &entries {
                let held = self.ships.entry(signature(entry, SIGNATURE_SEED)).or_insert((0, signature(entry, CHECK_SEED)));
                held.0 += 1;
            }
        }
        self.convoys.clear();
        if self.kinds[1].counted_from.is_some() {
            let view = hour.table(TableId::SunkConvoys);
            self.convoys.extend(view.rows().filter_map(|slot| convoy(&view, slot).ok()).map(|(key, _, _, count)| (key, count)));
        }
        self.last = Some(hour.hour);
    }

    fn save(&self, blob: &mut Bytes) -> io::Result<()> {
        for kind in &self.kinds {
            match kind.unread {
                None => blob.u8(0)?,
                Some((first, last)) => {
                    blob.u8(1)?;
                    blob.varint(u64::from(first as u32))?;
                    blob.varint(u64::from(last as u32))?;
                }
            }
        }
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        *self = Self::default();
        let mut reader = Reader::new(blob);
        let hour = |reader: &mut Reader<'_>| -> io::Result<i32> { Ok(u32::try_from(reader.varint()?).map_err(|_| invalid("NavalLosses' state names an hour past 32 bits"))? as i32) };
        for kind in &mut self.kinds {
            if reader.u8()? != 0 {
                kind.unread = Some((hour(&mut reader)?, hour(&mut reader)?));
            }
        }
        reader.finish()
    }
}
