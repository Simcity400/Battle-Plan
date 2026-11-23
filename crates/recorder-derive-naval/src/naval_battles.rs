
use recorder_derive_framework::framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer};
use replay_columnar::{
    leaf::{Reader, Writer as Bytes},
    manifest::Status,
};
use replay_data::rows::{NavalResultObservationRow as Observation, TableId};
use std::{
    collections::{HashMap, HashSet},
    io,
};

mod inputs;
#[cfg(test)]
mod tests;

use inputs::*;

const READS: &[Read] = &[
    Read { table: TableId::NavalCombats, columns: Columns::These(&PROVINCE), watch: Watch::Changed, required: true },
    Read { table: TableId::NavalCombatParticipants, columns: Columns::These(&MEMBER), watch: Watch::Changed, required: false },
    Read { table: TableId::NavalResults, columns: Columns::These(&[]), watch: Watch::Changed, required: false },
    Read { table: TableId::NavalResultSources, columns: Columns::Every, watch: Watch::Appended, required: false },
];

const TABLES: &[TableId] = &[TableId::NavalBattles, TableId::NavalResultObservations];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Quantities {
    ships: u32,
    convoys: u32,
    sunk: u32,
    convoys_sunk: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct Counts {
    quantities: Quantities,
    unknown: u32,
}

impl Counts {
    fn change(&mut self, flags: Option<(bool, bool)>, add: bool) {
        let step = |value: &mut u32| *value = if add { value.saturating_add(1) } else { value.saturating_sub(1) };
        let held = &mut self.quantities;
        match flags {
            None => step(&mut self.unknown),
            Some((true, sunk)) => {
                step(&mut held.convoys);
                if sunk {
                    step(&mut held.convoys_sunk);
                }
            }
            Some((false, sunk)) => {
                step(&mut held.ships);
                if sunk {
                    step(&mut held.sunk);
                }
            }
        }
    }

    fn quantities(&self) -> Option<Quantities> {
        (self.unknown == 0).then_some(self.quantities)
    }
}

fn highest(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        _ => a.or(b),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Sample {
    sides: [Option<Quantities>; 2],
    peak: Option<(u32, i32)>,
    sunk: [Option<u32>; 2],
    convoys_sunk: [Option<u32>; 2],
}

impl Sample {
    fn next(previous: Option<&Sample>, sides: [Option<Quantities>; 2], hour: i32) -> Self {
        let current = sides[0].zip(sides[1]).map(|(a, b)| a.ships.saturating_add(b.ships));
        let peak = match (previous.and_then(|previous| previous.peak), current) {
            (Some(held), Some(now)) if now > held.0 => Some((now, hour)),
            (Some(held), _) => Some(held),
            (None, now) => now.map(|now| (now, hour)),
        };
        let floor = |side: usize, value: fn(&Quantities) -> u32, held: fn(&Sample) -> [Option<u32>; 2]| highest(sides[side].as_ref().map(value), previous.and_then(|previous| held(previous)[side]));
        Self {
            sides,
            peak,
            sunk: [0, 1].map(|side| floor(side, |q| q.sunk, |s| s.sunk)),
            convoys_sunk: [0, 1].map(|side| floor(side, |q| q.convoys_sunk, |s| s.convoys_sunk)),
        }
    }

    fn row(&self, first: i32) -> Vec<Value> {
        let number = |value: Option<u32>| value.map_or(Value::Unrecorded, Value::U32);
        vec![
            Value::I32(first),
            number(self.peak.map(|peak| peak.0)),
            self.peak.map_or(Value::Unrecorded, |peak| Value::I32(peak.1)),
            number(self.sunk[0]),
            number(self.sunk[1]),
            number(self.convoys_sunk[0]),
            number(self.convoys_sunk[1]),
        ]
    }
}

#[derive(Clone, Copy, Debug)]
struct Live {
    province: u32,
    first: i32,
    sample: Sample,
}

fn refused(combat: u64, refusal: u8) -> Vec<Value> {
    let mut cells = vec![Value::U8(refusal), Value::U64(combat)];
    cells.resize(15, Value::Unrecorded);
    cells
}

fn observed(combat: u64, live: &Live, sample: &Sample, last: i32) -> Vec<Value> {
    let number = |value: Option<u32>| value.map_or(Value::Unrecorded, Value::U32);
    let mut cells = vec![
        Value::U8(Observation::OBSERVED),
        Value::U64(combat),
        Value::U32(live.province),
        Value::I32(live.first),
        Value::I32(last),
        number(sample.peak.map(|peak| peak.0)),
        sample.peak.map_or(Value::Unrecorded, |peak| Value::I32(peak.1)),
    ];
    for side in sample.sides {
        cells.extend([number(side.map(|q| q.ships)), number(side.map(|q| q.convoys)), number(side.map(|q| q.sunk)), number(side.map(|q| q.convoys_sunk))]);
    }
    cells
}

#[derive(Default)]
pub struct NavalBattles {
    live: HashMap<u64, Live>,
    members: HashMap<u128, Option<(bool, bool)>>,
    counts: HashMap<u64, [Counts; 2]>,
    participants: bool,
    linked: HashSet<u64>,
    last_source: Option<(u64, i32)>,
    unconfirmed: Vec<u64>,
    last: Option<i32>,
}

impl NavalBattles {
    fn sides(&self, combat: u64) -> [Option<Quantities>; 2] {
        if !self.participants {
            return [None; 2];
        }
        self.counts.get(&combat).map_or([Some(Quantities::default()); 2], |counts| counts.map(|side| side.quantities()))
    }

    fn count(&mut self, key: u128, flags: Option<(bool, bool)>, add: bool) {
        let side = ((key >> 32) & 1) as usize;
        let counts = self.counts.entry(member_combat(key)).or_default();
        counts[side].change(flags, add);
    }

    fn read_members(&mut self, hour: &Hour<'_>) {
        self.members.clear();
        self.counts.clear();
        self.participants = hour.complete(TableId::NavalCombatParticipants);
        if !self.participants {
            return;
        }
        let view = hour.table(TableId::NavalCombatParticipants);
        for slot in view.rows() {
            let Some(key) = view.key(slot) else { continue };
            let Some(member) = member(&view, slot) else { continue };
            self.members.insert(key, member.flags);
            self.counts.entry(member.combat).or_default()[member.side].change(member.flags, true);
        }
    }

    fn move_members(&mut self, hour: &Hour<'_>) -> HashSet<u64> {
        let table = TableId::NavalCombatParticipants;
        let view = hour.table(table);
        let mut moved = HashSet::new();
        for key in hour.deleted_keys(table) {
            if view.slot(key).is_none()
                && let Some(flags) = self.members.remove(&key)
            {
                self.count(key, flags, false);
                moved.insert(member_combat(key));
            }
        }
        for slot in hour.written_rows(table, Columns::These(&MEMBER)) {
            let Some(member) = member(&view, slot) else { continue };
            let Some(key) = view.key(slot) else { continue };
            if let Some(flags) = self.members.insert(key, member.flags) {
                self.count(key, flags, false);
            }
            self.count(key, member.flags, true);
            moved.insert(member.combat);
        }
        self.counts.retain(|_, sides| sides.iter().any(|side| side.quantities != Quantities::default() || side.unknown != 0));
        moved
    }

    fn begin(&mut self, combat: u64, province: u32, hour: i32) {
        let sample = Sample::next(None, self.sides(combat), hour);
        self.live.insert(combat, Live { province, first: hour, sample });
    }

    fn observe(&mut self, hour: &Hour<'_>, out: &Rows<'_>, source: &Source, earlier: &HashMap<u64, Sample>, before: &HashMap<u64, Live>, written: &HashMap<u64, Vec<Value>>) -> Option<Vec<Value>> {
        let at = hour.hour;
        let valid = source.id != 0
            && self.last_source.is_none_or(|(id, hour)| source.id > id && source.game_hour >= hour)
            && source.game_hour <= at
            && self.last.is_none_or(|last| source.game_hour >= last)
            && source.created.is_none_or(|created| created <= source.game_hour)
            && (!source.merged || source.transient);
        if !valid {
            return Some(refused(source.combat, Observation::INVALID_SOURCE));
        }
        self.last_source = Some((source.id, source.game_hour));
        if source.transient {
            return None;
        }
        let results = hour.table(TableId::NavalResults);
        let retained = results.slot(u128::from(source.result)).is_some_and(|slot| !hour.changes(TableId::NavalResults).inserted.contains(&slot));
        let seen = out.table(TableId::NavalResultObservations).slot(u128::from(source.result)).is_some() || written.contains_key(&source.result);
        if !self.linked.insert(source.combat) || seen || retained {
            return Some(refused(source.combat, Observation::DUPLICATE));
        }
        let (live, sample, read) = match self.live.get(&source.combat) {
            Some(live) => {
                let sample = if source.game_hour >= at { live.sample } else { earlier.get(&source.combat).copied().unwrap_or(live.sample) };
                (*live, sample, at)
            }
            None => match before.get(&source.combat) {
                Some(live) => (*live, live.sample, at - 1),
                None => return Some(refused(source.combat, Observation::NOT_OBSERVED)),
            },
        };
        if live.first > source.game_hour || source.created.is_some_and(|created| created > live.first) {
            return Some(refused(source.combat, Observation::MISMATCH));
        }
        Some(observed(source.combat, &live, &sample, source.game_hour.min(read)))
    }
}

impl Writer for NavalBattles {
    fn name(&self) -> &'static str {
        "NavalBattles"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::NavalBattles), Output::State(TableId::NavalResultObservations)]
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
        for &table in TABLES {
            out.status(table, Status::Complete);
        }
        let at = hour.hour;
        let fresh = hour.fresh || self.last.is_none_or(|last| last.checked_add(1) != Some(at));
        let participants = TableId::NavalCombatParticipants;
        let every = fresh || hour.complete(participants) != self.participants || hour.changes(participants).status_changed;
        let moved = if every {
            self.read_members(hour);
            HashSet::new()
        } else if self.participants {
            self.move_members(hour)
        } else {
            HashSet::new()
        };
        let combats = hour.table(TableId::NavalCombats);
        let mut before: HashMap<u64, Live> = HashMap::new();
        let mut earlier: HashMap<u64, Sample> = HashMap::new();
        let mut write: HashSet<u64> = HashSet::new();
        if fresh {
            self.live.clear();
            for slot in combats.rows() {
                let Some(combat) = combats.key(slot).map(|key| key as u64) else { continue };
                self.begin(combat, province(&combats, slot)?, at);
                write.insert(combat);
            }
            for key in recorder_derive_framework::forces::held(out, TableId::NavalBattles) {
                if !self.live.contains_key(&(key as u64)) {
                    out.delete(TableId::NavalBattles, key);
                }
            }
        } else {
            for key in hour.deleted_keys(TableId::NavalCombats) {
                if combats.slot(key).is_none()
                    && let Some(live) = self.live.remove(&(key as u64))
                {
                    before.insert(key as u64, live);
                }
            }
            let mut begun = HashSet::new();
            for slot in hour.written_rows(TableId::NavalCombats, Columns::These(&PROVINCE)) {
                let Some(combat) = combats.key(slot).map(|key| key as u64) else { continue };
                let province = province(&combats, slot)?;
                if self.live.get(&combat).is_some_and(|live| live.province == province) {
                    continue;
                }
                if let Some(old) = self.live.remove(&combat) {
                    before.insert(combat, old);
                }
                self.begin(combat, province, at);
                begun.insert(combat);
                write.insert(combat);
            }
            let resample: Vec<u64> = if every { self.live.keys().copied().collect() } else { moved.into_iter().filter(|combat| self.live.contains_key(combat)).collect() };
            for combat in resample {
                if begun.contains(&combat) {
                    continue;
                }
                let sides = self.sides(combat);
                let live = self.live.get_mut(&combat).expect("a live battle");
                let sample = Sample::next(Some(&live.sample), sides, at);
                if sample != live.sample {
                    earlier.insert(combat, live.sample);
                    if (sample.peak, sample.sunk, sample.convoys_sunk) != (live.sample.peak, live.sample.sunk, live.sample.convoys_sunk) {
                        write.insert(combat);
                    }
                    live.sample = sample;
                }
            }
            for &combat in before.keys() {
                if !self.live.contains_key(&combat) {
                    out.delete(TableId::NavalBattles, u128::from(combat));
                }
            }
        }
        for &combat in &write {
            if let Some(live) = self.live.get(&combat) {
                out.set(TableId::NavalBattles, u128::from(combat), live.sample.row(live.first));
            }
        }
        let results_complete = hour.complete(TableId::NavalResults);
        if results_complete {
            let results = hour.table(TableId::NavalResults);
            let looked: Vec<u128> = if fresh || hour.changes(TableId::NavalResults).status_changed {
                recorder_derive_framework::forces::held(out, TableId::NavalResultObservations)
            } else {
                hour.deleted_keys(TableId::NavalResults).chain(self.unconfirmed.iter().map(|&result| u128::from(result))).collect()
            };
            for key in looked {
                if results.slot(key).is_none() && out.table(TableId::NavalResultObservations).slot(key).is_some() {
                    out.delete(TableId::NavalResultObservations, key);
                }
            }
            self.unconfirmed.clear();
        }
        let mut written: HashMap<u64, Vec<Value>> = HashMap::new();
        for source in sources(hour)?.unwrap_or_default() {
            if let Some(cells) = self.observe(hour, out, &source, &earlier, &before, &written) {
                written.insert(source.result, cells);
            }
        }
        let results = hour.table(TableId::NavalResults);
        self.unconfirmed.extend(written.keys().copied().filter(|&result| !results_complete || results.slot(u128::from(result)).is_none()));
        for (result, cells) in written {
            out.set(TableId::NavalResultObservations, u128::from(result), cells);
        }
        self.unconfirmed.sort_unstable();
        self.unconfirmed.dedup();
        self.linked.retain(|combat| self.live.contains_key(combat));
        self.last = Some(at);
        Ok(())
    }

    fn prime(&mut self, hour: &Hour<'_>) {
        self.live.clear();
        self.read_members(hour);
        let held = hour.table(TableId::NavalBattles);
        let combats = hour.table(TableId::NavalCombats);
        for slot in held.rows() {
            let Some(combat) = held.key(slot).map(|key| key as u64) else { continue };
            let (Some(row), Some(province)) = (inputs::held(&held, slot), combats.slot(u128::from(combat)).and_then(|slot| province(&combats, slot).ok())) else { continue };
            let sample = Sample { sides: self.sides(combat), peak: row.peak, sunk: row.sunk, convoys_sunk: row.convoys_sunk };
            self.live.insert(combat, Live { province, first: row.first, sample });
        }
        self.last = Some(hour.hour);
    }

    fn save(&self, blob: &mut Bytes) -> io::Result<()> {
        match self.last_source {
            None => blob.u8(0)?,
            Some((id, hour)) => {
                blob.u8(1)?;
                blob.varint(id)?;
                blob.varint(u64::from(hour as u32))?;
            }
        }
        let mut linked: Vec<u64> = self.linked.iter().copied().collect();
        linked.sort_unstable();
        for list in [&linked, &self.unconfirmed] {
            blob.varint(list.len() as u64)?;
            for &value in list {
                blob.varint(value)?;
            }
        }
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        *self = Self::default();
        let mut reader = Reader::new(blob);
        if reader.u8()? != 0 {
            let id = reader.varint()?;
            let hour = u32::try_from(reader.varint()?).map_err(|_| invalid("NavalBattles' state names an hour past 32 bits"))? as i32;
            self.last_source = Some((id, hour));
        }
        let mut lists = [Vec::new(), Vec::new()];
        for list in &mut lists {
            let count = reader.count()?;
            if count > blob.len() {
                return Err(invalid("NavalBattles' state counts more handles than it holds"));
            }
            for _ in 0..count {
                list.push(reader.varint()?);
            }
        }
        let [linked, unconfirmed] = lists;
        self.linked = linked.into_iter().collect();
        self.unconfirmed = unconfirmed;
        reader.finish()
    }
}
