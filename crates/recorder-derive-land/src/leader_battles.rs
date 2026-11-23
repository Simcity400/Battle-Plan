
mod inputs;
#[cfg(test)]
mod tests;

use recorder_derive_framework::framework::{Columns, Hour, Keys, Output, Read, Rows, Value, Watch, Writer};
use inputs::invalid;
use replay_columnar::{
    leaf::{Reader, Writer as Bytes},
    manifest::Status,
};
use replay_data::rows::{LeaderBattleRow, TableId};
use std::{
    collections::{HashMap, HashSet},
    io,
};

const READS: &[Read] = &[
    Read { table: TableId::LandCombatDetails, columns: Columns::These(&inputs::LAND_SIDES), watch: Watch::Changed, required: true },
    Read { table: TableId::NavalCombatDetails, columns: Columns::These(&inputs::NAVAL_SIDES), watch: Watch::Changed, required: true },
    Read { table: TableId::LandCombatResults, columns: Columns::Every, watch: Watch::Appended, required: false },
    Read { table: TableId::NavalResultSources, columns: Columns::Every, watch: Watch::Appended, required: false },
    Read { table: TableId::NavalResults, columns: Columns::These(&inputs::WINNER), watch: Watch::Changed, required: false },
    Read { table: TableId::NavalCombatResultDetails, columns: Columns::Every, watch: Watch::Changed, required: false },
    Read { table: TableId::UnitLeaders, columns: Columns::These(&[]), watch: Watch::Changed, required: false },
];

const LINK_HOURS: i32 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Commander {
    leader: u64,
    sides: u8,
    start: i32,
    unknown: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Combat {
    commanders: Vec<Commander>,
    ended: Option<i32>,
    result: Option<(i32, Option<u8>)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Link {
    combat: u64,
    date: i32,
    read: i32,
}

#[derive(Default)]
pub struct LeaderBattles {
    combats: HashMap<(u8, u64), Combat>,
    links: HashMap<u64, Link>,
}

struct Outcome {
    date: i32,
    winners: Option<u8>,
    named: [u64; 2],
}

struct Entry {
    leader: u64,
    kind: u8,
    combat: u64,
    entry: u8,
    cells: Vec<Value>,
}

fn participation(kind: u8, combat: u64, commander: &Commander) -> Entry {
    Entry {
        leader: commander.leader,
        kind,
        combat,
        entry: LeaderBattleRow::PARTICIPATION,
        cells: vec![
            Value::U8(LeaderBattleRow::PARTICIPATION),
            Value::U8(kind),
            Value::U64(combat),
            Value::U8(commander.sides),
            Value::I32(commander.start),
            Value::Bool(commander.unknown),
            Value::Unrecorded,
            Value::Unrecorded,
            Value::Unrecorded,
        ],
    }
}

fn result(kind: u8, combat: u64, commander: &Commander, date: i32, winners: Option<u8>) -> Entry {
    let outcome = winners.filter(|&won| won != 0 && commander.sides.count_ones() == 1 && date >= commander.start).map(|won| if won & commander.sides != 0 { LeaderBattleRow::WON } else { LeaderBattleRow::LOST });
    Entry {
        leader: commander.leader,
        kind,
        combat,
        entry: LeaderBattleRow::RESULT,
        cells: vec![
            Value::U8(LeaderBattleRow::RESULT),
            Value::U8(kind),
            Value::U64(combat),
            Value::U8(commander.sides),
            Value::I32(commander.start),
            Value::Bool(commander.unknown),
            winners.map_or(Value::Unrecorded, Value::U8),
            Value::I32(date),
            outcome.map_or(Value::Unrecorded, Value::U8),
        ],
    }
}

fn command(combat: &mut Combat, kind: u8, handle: u64, leader: u64, side: usize, start: i32, unknown: bool) -> Option<Entry> {
    if leader == 0 {
        return None;
    }
    let bit = 1 << side;
    match combat.commanders.iter_mut().find(|held| held.leader == leader) {
        Some(held) => {
            held.sides |= bit;
            None
        }
        None => {
            let commander = Commander { leader, sides: bit, start, unknown };
            combat.commanders.push(commander);
            Some(participation(kind, handle, &commander))
        }
    }
}

fn counted(entry: &Entry) -> [u32; 3] {
    match (entry.entry, entry.cells.get(8)) {
        (LeaderBattleRow::PARTICIPATION, _) => [1, 0, 0],
        (_, Some(Value::U8(LeaderBattleRow::WON))) => [0, 1, 0],
        (_, Some(Value::U8(LeaderBattleRow::LOST))) => [0, 0, 1],
        _ => [0; 3],
    }
}

fn total(hour: &Hour<'_>, out: &mut Rows, entries: &[Entry]) {
    let table = TableId::LeaderBattleTotals;
    let leaders = hour.table(TableId::UnitLeaders);
    let gone: Vec<u128> = match hour.fresh {
        true => Vec::new(),
        false => hour.deleted_keys(TableId::UnitLeaders).filter(|&key| leaders.slot(key).is_none()).collect(),
    };
    let mut added: HashMap<u64, [u32; 3]> = HashMap::new();
    for entry in entries {
        let counts = added.entry(entry.leader).or_default();
        for (held, add) in counts.iter_mut().zip(counted(entry)) {
            *held += add;
        }
    }
    let mut added: Vec<(u64, [u32; 3])> = added.into_iter().filter(|(leader, _)| !gone.contains(&u128::from(*leader))).collect();
    added.sort_unstable();
    for (leader, add) in added {
        let key = u128::from(leader);
        let held = out.table(table);
        let before = held.slot(key).map_or([0; 3], |slot| [0, 1, 2].map(|column| held.number(column, slot).unwrap_or(0) as u32));
        let after: Vec<Value> = before.iter().zip(add).map(|(held, add)| Value::U32(held.saturating_add(add))).collect();
        out.set(table, key, after);
    }
    for key in gone {
        if out.table(table).slot(key).is_some() {
            out.delete(table, key);
        }
    }
}

impl LeaderBattles {
    fn live(&mut self, hour: &Hour<'_>, kind: u8, entries: &mut Vec<Entry>) -> io::Result<()> {
        let naval = kind == LeaderBattleRow::NAVAL;
        let (table, sides) = if naval { (TableId::NavalCombatDetails, &inputs::NAVAL_SIDES) } else { (TableId::LandCombatDetails, &inputs::LAND_SIDES) };
        let view = hour.table(table);
        let at = hour.hour;
        let slots: Vec<u32> = if hour.fresh {
            for (&(held, handle), combat) in &mut self.combats {
                if held == kind && combat.ended.is_none() && view.slot(u128::from(handle)).is_none() {
                    combat.ended = Some(at);
                }
            }
            view.rows().collect()
        } else {
            for key in hour.deleted_keys(table) {
                if view.slot(key).is_none()
                    && let Some(combat) = self.combats.get_mut(&(kind, key as u64))
                {
                    combat.ended = Some(at);
                }
            }
            hour.written_rows(table, Columns::These(sides))
        };
        let inserted: HashSet<u32> = if hour.fresh { HashSet::new() } else { hour.changes(table).inserted.iter().copied().collect() };
        for slot in slots {
            let handle = view.key(slot).ok_or_else(|| invalid(format!("{table:?} slot {slot} holds no row")))? as u64;
            let commanders = inputs::commanders(hour, naval, slot)?;
            let combat = self.combats.entry((kind, handle)).or_default();
            if combat.ended.is_some() {
                *combat = Combat::default();
            }
            let unknown = !inserted.contains(&slot);
            for (side, leader) in commanders.into_iter().enumerate() {
                if let Some(entry) = command(combat, kind, handle, leader, side, at, unknown) {
                    entries.push(entry);
                }
            }
        }
        Ok(())
    }

    fn resulted(&mut self, kind: u8, handle: u64, outcome: Outcome, at: i32, entries: &mut Vec<Entry>) {
        let combat = self.combats.entry((kind, handle)).or_insert_with(|| Combat { ended: Some(at), ..Combat::default() });
        let Outcome { date, winners, named } = outcome;
        let winners = match combat.result {
            Some(held) if held == (date, winners) => return,
            Some(_) => None,
            None => winners,
        };
        combat.result = Some((date, winners));
        for (side, leader) in named.into_iter().enumerate() {
            if let Some(entry) = command(combat, kind, handle, leader, side, date, true) {
                entries.push(entry);
            }
        }
        for commander in &combat.commanders {
            entries.push(result(kind, handle, commander, date, winners));
        }
    }

    fn results(&mut self, hour: &Hour<'_>, entries: &mut Vec<Entry>) -> io::Result<()> {
        let at = hour.hour;
        if hour.complete(TableId::LandCombatResults) {
            for logged in hour.appended(TableId::LandCombatResults).rows() {
                let land = inputs::land_result(hour, logged.slot)?;
                let outcome = Outcome { date: land.hour, winners: (land.winners <= 3).then_some(land.winners), named: land.commanders };
                self.resulted(LeaderBattleRow::LAND, land.combat, outcome, at, entries);
            }
        }
        if hour.complete(TableId::NavalResultSources) {
            for logged in hour.appended(TableId::NavalResultSources).rows() {
                let source = inputs::naval_source(hour, logged.slot)?;
                if !source.transient && !source.merged {
                    self.links.insert(source.result, Link { combat: source.combat, date: source.game_hour, read: at });
                }
            }
        }
        let mut ready: Vec<u64> = self.links.keys().copied().filter(|&result| inputs::naval_winner(hour, result).is_some()).collect();
        ready.sort_unstable();
        for result in ready {
            let Some(link) = self.links.remove(&result) else { continue };
            let winners = inputs::naval_winner(hour, result).flatten().filter(|&side| side <= 2);
            let outcome = Outcome { date: link.date, winners, named: inputs::naval_result_leaders(hour, result)? };
            self.resulted(LeaderBattleRow::NAVAL, link.combat, outcome, at, entries);
        }
        self.links.retain(|_, link| at - link.read < LINK_HOURS);
        Ok(())
    }
}

impl Writer for LeaderBattles {
    fn name(&self) -> &'static str {
        "LeaderBattles"
    }

    fn version(&self) -> u32 {
        2
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::Log(TableId::LeaderBattles, Keys::Hour(32)), Output::State(TableId::LeaderBattleTotals)]
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
        for table in [TableId::LeaderBattles, TableId::LeaderBattleTotals] {
            out.status(table, Status::Complete);
        }
        recorder_derive_framework::forces::forget_listed(hour, out, TableId::LeaderBattles);
        let at = hour.hour;
        let mut entries = Vec::new();
        self.live(hour, LeaderBattleRow::LAND, &mut entries)?;
        self.live(hour, LeaderBattleRow::NAVAL, &mut entries)?;
        self.results(hour, &mut entries)?;
        self.combats.retain(|_, combat| combat.ended.is_none_or(|ended| ended >= at));
        if hour.first {
            return Ok(());
        }
        total(hour, out, &entries);
        entries.sort_unstable_by_key(|entry| (entry.leader, entry.kind, entry.combat, entry.entry));
        let mut ordinal = 0u32;
        let mut previous = None;
        for entry in entries {
            if previous != Some(entry.leader) {
                ordinal = 0;
                previous = Some(entry.leader);
            }
            let key = u128::from(entry.leader) << 64 | u128::from(at as u32) << 32 | u128::from(ordinal);
            out.append(TableId::LeaderBattles, key, entry.cells);
            ordinal += 1;
        }
        Ok(())
    }

    fn save(&self, blob: &mut Bytes) -> io::Result<()> {
        let hour = |blob: &mut Bytes, hour: i32| blob.raw(&hour.to_le_bytes());
        let mut combats: Vec<(&(u8, u64), &Combat)> = self.combats.iter().collect();
        combats.sort_unstable_by_key(|(key, _)| **key);
        blob.varint(combats.len() as u64)?;
        for (&(kind, handle), combat) in combats {
            blob.u8(kind)?;
            blob.varint(handle)?;
            match combat.ended {
                Some(ended) => {
                    blob.u8(1)?;
                    hour(blob, ended)?;
                }
                None => blob.u8(0)?,
            }
            match combat.result {
                Some((date, winners)) => {
                    blob.u8(1)?;
                    hour(blob, date)?;
                    blob.u8(winners.map_or(u8::MAX, |won| won))?;
                }
                None => blob.u8(0)?,
            }
            blob.varint(combat.commanders.len() as u64)?;
            for commander in &combat.commanders {
                blob.varint(commander.leader)?;
                blob.u8(commander.sides)?;
                hour(blob, commander.start)?;
                blob.u8(u8::from(commander.unknown))?;
            }
        }
        let mut links: Vec<(&u64, &Link)> = self.links.iter().collect();
        links.sort_unstable_by_key(|(result, _)| **result);
        blob.varint(links.len() as u64)?;
        for (result, link) in links {
            blob.varint(*result)?;
            blob.varint(link.combat)?;
            hour(blob, link.date)?;
            hour(blob, link.read)?;
        }
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        *self = Self::default();
        let mut reader = Reader::new(blob);
        let hour = |reader: &mut Reader<'_>| -> io::Result<i32> { Ok(i32::from_le_bytes(reader.raw(4)?.try_into().map_err(|_| invalid("LeaderBattles' state holds a cut hour"))?)) };
        let count = reader.count()?;
        if count > blob.len() {
            return Err(invalid("LeaderBattles' state counts more combats than it holds"));
        }
        for _ in 0..count {
            let kind = reader.u8()?;
            let handle = reader.varint()?;
            let ended = match reader.u8()? {
                0 => None,
                _ => Some(hour(&mut reader)?),
            };
            let result = match reader.u8()? {
                0 => None,
                _ => {
                    let date = hour(&mut reader)?;
                    let winners = reader.u8()?;
                    Some((date, (winners != u8::MAX).then_some(winners)))
                }
            };
            let commanders = reader.count()?;
            if commanders > blob.len() {
                return Err(invalid("LeaderBattles' state counts more commanders than it holds"));
            }
            let mut combat = Combat { commanders: Vec::with_capacity(commanders), ended, result };
            for _ in 0..commanders {
                let leader = reader.varint()?;
                let sides = reader.u8()?;
                let start = hour(&mut reader)?;
                let unknown = reader.u8()? != 0;
                combat.commanders.push(Commander { leader, sides, start, unknown });
            }
            self.combats.insert((kind, handle), combat);
        }
        let count = reader.count()?;
        if count > blob.len() {
            return Err(invalid("LeaderBattles' state counts more links than it holds"));
        }
        for _ in 0..count {
            let result = reader.varint()?;
            let combat = reader.varint()?;
            let date = hour(&mut reader)?;
            let read = hour(&mut reader)?;
            self.links.insert(result, Link { combat, date, read });
        }
        reader.finish()
    }
}
