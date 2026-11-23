
use recorder_derive_framework::framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer};
use replay_columnar::{leaf::Writer as Bytes, manifest::Status};
use replay_data::{
    rows::{LandBattleEndRow as End, LandEquipmentLoss, TableId},
    value::EncodeValue,
};
use std::{
    collections::{HashMap, HashSet},
    io,
};

mod inputs;
#[cfg(test)]
mod tests;

use inputs::*;

const READS: &[Read] = &[
    Read { table: TableId::LandCombats, columns: Columns::These(&COMBAT), watch: Watch::Changed, required: true },
    Read { table: TableId::LandCombatSides, columns: Columns::These(&SIDES), watch: Watch::Changed, required: false },
    Read { table: TableId::LandCombatProgress, columns: Columns::These(&PROGRESS), watch: Watch::Changed, required: false },
    Read { table: TableId::LandCombatResults, columns: Columns::Every, watch: Watch::Appended, required: false },
    Read { table: TableId::LandEventCoverage, columns: Columns::Every, watch: Watch::Changed, required: false },
    Read { table: TableId::CountryLinks, columns: Columns::These(&LINK_SLOT), watch: Watch::Changed, required: false },
];

const TABLES: &[TableId] = &[TableId::LandBattles, TableId::LandBattleEnds];

const BOTH_UNKNOWN: u8 = End::ATTACKERS_UNKNOWN | End::DEFENDERS_UNKNOWN;
const SIDE_UNKNOWN: [u8; 2] = [End::ATTACKERS_UNKNOWN, End::DEFENDERS_UNKNOWN];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Peak {
    value: u32,
    hour: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Countries {
    slots: [Vec<u32>; 2],
    unknown: u8,
}

impl Countries {
    fn unknown() -> Self {
        Self { slots: [Vec::new(), Vec::new()], unknown: BOTH_UNKNOWN }
    }
}

fn resolve(hour: &Hour<'_>, raw: Option<&[Vec<u32>; 2]>) -> Countries {
    let Some(raw) = raw else { return Countries::unknown() };
    let mut out = Countries { slots: [Vec::new(), Vec::new()], unknown: 0 };
    for side in 0..2 {
        for &reference in &raw[side] {
            match link(hour, reference) {
                Some(slot) => out.slots[side].push(slot),
                None => out.unknown |= SIDE_UNKNOWN[side],
            }
        }
        out.slots[side].sort_unstable();
        out.slots[side].dedup();
        if out.slots[side].is_empty() {
            out.unknown |= SIDE_UNKNOWN[side];
        }
    }
    out
}

#[derive(Clone, Debug)]
struct Battle {
    start: i32,
    exact: bool,
    counter_base: i32,
    province: Option<u32>,
    duration: Option<i32>,
    peaks: [Option<Peak>; 3],
    manpower: [Option<i32>; 2],
    raw: Option<[Vec<u32>; 2]>,
    countries: Countries,
}

impl Battle {
    fn new(start: i32, exact: bool, combat: &Combat) -> Self {
        let mut battle = Self {
            start,
            exact,
            counter_base: if exact { 0 } else { combat.duration },
            province: Some(combat.province),
            duration: Some(combat.duration),
            peaks: [None; 3],
            manpower: [None; 2],
            raw: None,
            countries: Countries::unknown(),
        };
        battle.raise(combat.counts, start);
        battle
    }

    fn of(row: &inputs::Held) -> Self {
        Self {
            start: row.start,
            exact: row.exact,
            counter_base: row.counter_base,
            province: None,
            duration: None,
            peaks: row.peaks.map(|peak| peak.map(|(value, hour)| Peak { value, hour })),
            manpower: [None; 2],
            raw: None,
            countries: Countries::unknown(),
        }
    }

    fn goes_on(&self, combat: &Combat) -> bool {
        self.province == Some(combat.province) && self.duration.is_some_and(|duration| combat.duration >= duration)
    }

    fn raise(&mut self, counts: Option<[u32; 2]>, hour: i32) -> bool {
        let Some([attackers, defenders]) = counts else { return false };
        let mut raised = false;
        for (peak, value) in self.peaks.iter_mut().zip([attackers.saturating_add(defenders), attackers, defenders]) {
            if peak.is_none_or(|peak| value > peak.value) {
                *peak = Some(Peak { value, hour });
                raised = true;
            }
        }
        raised
    }

    fn peak_cells(&self) -> Vec<Value> {
        self.peaks
            .iter()
            .flat_map(|peak| match peak {
                Some(peak) => [Value::U32(peak.value), Value::I32(peak.hour)],
                None => [Value::Unrecorded; 2],
            })
            .collect()
    }

    fn row(&self) -> Vec<Value> {
        let mut cells = vec![Value::I32(self.start), Value::Bool(self.exact), Value::I32(self.counter_base)];
        cells.extend(self.peak_cells());
        cells
    }
}

fn optional(value: Option<i32>) -> Value {
    value.map_or(Value::Unrecorded, Value::I32)
}

struct Outcome {
    id: u64,
    combat: u64,
    province: u32,
    end_hour: i32,
    duration: i32,
    winners: u8,
    manpower: [i32; 2],
    equipment: [Vec<LandEquipmentLoss>; 2],
    countries: Countries,
}

impl Outcome {
    fn of(hour: &Hour<'_>, finalizer: &Finalizer) -> Self {
        Self {
            id: finalizer.id,
            combat: finalizer.combat,
            province: finalizer.province,
            end_hour: finalizer.end_hour,
            duration: finalizer.duration,
            winners: finalizer.winners,
            manpower: finalizer.manpower,
            equipment: finalizer.equipment.clone(),
            countries: resolve(hour, Some(&finalizer.countries)),
        }
    }

    fn cells(outcomes: &[Self], out: &mut Rows<'_>) -> [Value; 9] {
        let first = &outcomes[0];
        let alike = |same: fn(&Self, &Self) -> bool| outcomes.iter().all(|other| same(first, other));
        let known = |alike: bool, value: Value| if alike { value } else { Value::Unrecorded };
        let mut list = |side: usize| if outcomes.iter().all(|other| other.countries.slots[side] == first.countries.slots[side]) { out.list(first.countries.slots[side].clone()) } else { Value::Unrecorded };
        let lists = [list(0), list(1)];
        [
            known(alike(|a, b| a.id == b.id), Value::OptionalU64(Some(first.id))),
            Value::Bool(true),
            known(alike(|a, b| a.winners == b.winners), Value::U8(first.winners)),
            known(alike(|a, b| a.duration == b.duration), Value::I32(first.duration)),
            known(alike(|a, b| a.manpower[0] == b.manpower[0]), Value::I32(first.manpower[0])),
            known(alike(|a, b| a.manpower[1] == b.manpower[1]), Value::I32(first.manpower[1])),
            lists[0],
            lists[1],
            known(alike(|a, b| a.countries.unknown == b.countries.unknown), Value::U8(first.countries.unknown)),
        ]
    }

    fn equipment(outcomes: &[Self], out: &mut Rows<'_>) -> io::Result<[Value; 2]> {
        let mut cells = [Value::Unrecorded, Value::Unrecorded];
        for (side, cell) in cells.iter_mut().enumerate() {
            let pool = &outcomes[0].equipment[side];
            if outcomes.iter().all(|other| &other.equipment[side] == pool) {
                let mut bytes = Vec::new();
                pool.encode_value(&mut bytes)?;
                *cell = out.opaque(bytes);
            }
        }
        Ok(cells)
    }
}

struct Ended {
    combat: u64,
    battle: Battle,
    kind: u8,
    found: i32,
    last_read: Option<i32>,
    outcome: Option<Outcome>,
}

fn ends(battle: &Battle, read: Option<i32>, finalizer: &Finalizer, from: i32) -> bool {
    let Some(read) = read else { return false };
    battle.province == Some(finalizer.province)
        && battle.start.saturating_sub(1) <= finalizer.end_hour
        && read >= from
        && read.saturating_sub(1) <= finalizer.end_hour
        && read.checked_add(1).is_some_and(|next| finalizer.end_hour <= next)
}

fn end_key(combat: u64, result_only: bool, start: i32) -> u128 {
    u128::from(combat) << 33 | u128::from(result_only) << 32 | u128::from(start as u32)
}

#[derive(Default)]
pub struct LandBattles {
    live: HashMap<u64, Battle>,
    finalized: Vec<u64>,
    living: Vec<u128>,
    last: Option<i32>,
}

impl LandBattles {
    fn begin(&mut self, hour: &Hour<'_>, combat: u64, exact: bool, read: &Combat) -> io::Result<()> {
        let mut battle = Battle::new(hour.hour, exact, read);
        battle.raw = sides(hour, combat)?;
        battle.countries = resolve(hour, battle.raw.as_ref());
        battle.manpower = manpower(hour, combat);
        self.live.insert(combat, battle);
        Ok(())
    }

    fn rebuild(&mut self, hour: &Hour<'_>, out: &Rows<'_>, ended: &mut Vec<Ended>) -> io::Result<()> {
        let at = hour.hour;
        let held = out.table(TableId::LandBattles);
        if !hour.first {
            let last_read = self.last.or(held.sampled_at());
            let found = last_read.and_then(|read| read.checked_add(1)).unwrap_or(at);
            for slot in held.rows() {
                let Some(combat) = held.key(slot).map(|key| key as u64) else { continue };
                let battle = match self.live.remove(&combat) {
                    Some(battle) => battle,
                    None => match inputs::held(&held, slot) {
                        Some(row) => Battle::of(&row),
                        None => continue,
                    },
                };
                ended.push(Ended { combat, battle, kind: End::UNKNOWN, found, last_read, outcome: None });
            }
        }
        self.live.clear();
        self.finalized.clear();
        let combats = hour.table(TableId::LandCombats);
        for slot in combats.rows() {
            let Some(combat) = combats.key(slot).map(|key| key as u64) else { continue };
            let read = inputs::combat(&combats, slot, true)?;
            self.begin(hour, combat, false, &read)?;
        }
        Ok(())
    }

    fn census(&mut self, hour: &Hour<'_>, ended: &mut Vec<Ended>) -> io::Result<(HashSet<u64>, HashSet<u64>)> {
        let at = hour.hour;
        let previous = Some(at - 1);
        let combats = hour.table(TableId::LandCombats);
        let (mut written, mut begun) = (HashSet::new(), HashSet::new());
        for &(_, key) in &hour.changes(TableId::LandCombats).deleted {
            let combat = key as u64;
            if combats.slot(key).is_none()
                && let Some(battle) = self.live.remove(&combat)
            {
                ended.push(Ended { combat, battle, kind: End::ABSENT, found: at, last_read: previous, outcome: None });
            }
        }
        let changes = hour.changes(TableId::LandCombats);
        let rosters: HashSet<u32> = changes.written.iter().filter(|(_, column)| COMBAT[2..].contains(&usize::from(*column))).map(|&(slot, _)| slot).chain(changes.inserted.iter().copied()).collect();
        let mut visit = hour.written_rows(TableId::LandCombats, Columns::These(&COMBAT));
        visit.extend(self.finalized.drain(..).filter_map(|combat| combats.slot(u128::from(combat))));
        visit.sort_unstable();
        visit.dedup();
        for slot in visit {
            let Some(combat) = combats.key(slot).map(|key| key as u64) else { continue };
            let read = inputs::combat(&combats, slot, rosters.contains(&slot))?;
            match self.live.get_mut(&combat) {
                Some(battle) if battle.goes_on(&read) => {
                    battle.duration = Some(read.duration);
                    if battle.raise(read.counts, at) {
                        written.insert(combat);
                    }
                    continue;
                }
                Some(_) => {
                    let battle = self.live.remove(&combat).expect("a live battle");
                    ended.push(Ended { combat, battle, kind: End::REUSED, found: at, last_read: previous, outcome: None });
                }
                None => {}
            }
            self.begin(hour, combat, true, &inputs::combat(&combats, slot, true)?)?;
            written.insert(combat);
            begun.insert(combat);
        }
        Ok((written, begun))
    }

    fn counters(&mut self, hour: &Hour<'_>, begun: &HashSet<u64>) -> io::Result<()> {
        let links = hour.changes(TableId::CountryLinks).status_changed || hour.wrote(TableId::CountryLinks, Columns::These(&LINK_SLOT));
        let tables: [(TableId, &'static [usize]); 2] = [(TableId::LandCombatSides, &SIDES), (TableId::LandCombatProgress, &PROGRESS)];
        for (table, columns) in tables {
            let every = hour.changes(table).status_changed || (table == TableId::LandCombatSides && links);
            let combats: Vec<u64> = if every {
                self.live.keys().copied().collect()
            } else {
                let view = hour.table(table);
                let mut moved: Vec<u64> = hour.written_rows(table, Columns::These(columns)).into_iter().filter_map(|slot| view.key(slot)).map(|key| key as u64).collect();
                moved.extend(hour.deleted_keys(table).map(|key| key as u64));
                moved
            };
            for combat in combats {
                if begun.contains(&combat) {
                    continue;
                }
                let Some(battle) = self.live.get_mut(&combat) else { continue };
                if table == TableId::LandCombatSides {
                    battle.raw = sides(hour, combat)?;
                    battle.countries = resolve(hour, battle.raw.as_ref());
                } else {
                    battle.manpower = manpower(hour, combat);
                }
            }
        }
        Ok(())
    }

    fn finalize(&mut self, hour: &Hour<'_>, from: i32, ended: &mut Vec<Ended>) -> io::Result<Vec<Outcome>> {
        let at = hour.hour;
        let mut own = Vec::new();
        for finalizer in finalizers(hour)? {
            if finalizer.end_hour < from || finalizer.end_hour > at {
                continue;
            }
            let combat = finalizer.combat;
            if let Some(end) = ended.iter_mut().rev().find(|end| end.combat == combat && end.outcome.is_none() && ends(&end.battle, end.last_read, &finalizer, from)) {
                end.outcome = Some(Outcome::of(hour, &finalizer));
                continue;
            }
            if self.live.get(&combat).is_some_and(|battle| ends(battle, Some(at), &finalizer, from)) {
                let battle = self.live.remove(&combat).expect("a live battle");
                self.finalized.push(combat);
                let outcome = Some(Outcome::of(hour, &finalizer));
                ended.push(Ended { combat, battle, kind: End::FINALIZED, found: at, last_read: Some(at), outcome });
                continue;
            }
            own.push(Outcome::of(hour, &finalizer));
        }
        Ok(own)
    }

    fn end_row(&mut self, end: Ended, drained: bool, out: &mut Rows) -> io::Result<()> {
        let Ended { combat, battle, kind, found, last_read, outcome } = end;
        let key = end_key(combat, false, battle.start);
        let end_hour = outcome.as_ref().map_or(found, |outcome| outcome.end_hour);
        let mut cells = vec![
            Value::U8(kind),
            Value::I32(end_hour),
            Value::Bool(battle.exact),
            Value::I32(battle.counter_base),
            battle.province.map_or(Value::Unrecorded, Value::U32),
            Value::OptionalI32(last_read),
            optional(battle.duration),
        ];
        cells.extend(battle.peak_cells());
        cells.extend([optional(battle.manpower[0]), optional(battle.manpower[1])]);
        let lists = battle.raw.is_some();
        cells.extend([
            if lists { out.list(battle.countries.slots[0].clone()) } else { Value::Unrecorded },
            if lists { out.list(battle.countries.slots[1].clone()) } else { Value::Unrecorded },
            Value::U8(battle.countries.unknown),
        ]);
        match &outcome {
            Some(outcome) => cells.extend(Outcome::cells(std::slice::from_ref(outcome), out)),
            None => cells.extend([
                Value::OptionalU64(None),
                Value::Bool(drained && kind != End::UNKNOWN),
                Value::Unrecorded,
                Value::Unrecorded,
                Value::Unrecorded,
                Value::Unrecorded,
                Value::Unrecorded,
                Value::Unrecorded,
                Value::U8(BOTH_UNKNOWN),
            ]),
        }
        match &outcome {
            Some(outcome) => cells.extend(Outcome::equipment(std::slice::from_ref(outcome), out)?),
            None => cells.extend([Value::Unrecorded, Value::Unrecorded]),
        }
        out.set(TableId::LandBattleEnds, key, cells);
        self.living.push(key);
        Ok(())
    }

    fn own_row(&mut self, outcomes: &[Outcome], out: &mut Rows<'_>) -> io::Result<()> {
        let outcome = &outcomes[0];
        let key = end_key(outcome.combat, true, outcome.end_hour);
        let province = if outcomes.iter().all(|other| other.province == outcome.province) { Value::U32(outcome.province) } else { Value::Unrecorded };
        let mut cells = vec![Value::U8(End::RESULT_ONLY), Value::I32(outcome.end_hour), Value::Bool(false), Value::I32(0), province, Value::OptionalI32(None)];
        cells.extend([Value::Unrecorded; 9]);
        cells.extend([Value::Unrecorded, Value::Unrecorded, Value::U8(BOTH_UNKNOWN)]);
        cells.extend(Outcome::cells(outcomes, out));
        cells.extend(Outcome::equipment(outcomes, out)?);
        out.set(TableId::LandBattleEnds, key, cells);
        self.living.push(key);
        Ok(())
    }
}

impl Writer for LandBattles {
    fn name(&self) -> &'static str {
        "LandBattles"
    }

    fn version(&self) -> u32 {
        3
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::LandBattles), Output::State(TableId::LandBattleEnds)]
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
        for key in self.living.drain(..) {
            out.delete(TableId::LandBattleEnds, key);
        }
        let at = hour.hour;
        let fresh = hour.fresh || self.last.is_none_or(|last| last.checked_add(1) != Some(at));
        let moved = |table: TableId, columns: Columns| hour.changes(table).status_changed || hour.wrote(table, columns);
        let quiet = !moved(TableId::LandCombats, Columns::These(&COMBAT))
            && !moved(TableId::LandCombatSides, Columns::These(&SIDES))
            && !moved(TableId::LandCombatProgress, Columns::These(&PROGRESS))
            && !hour.changes(TableId::LandCombatResults).status_changed
            && hour.appended(TableId::LandCombatResults).is_empty()
            && !moved(TableId::LandEventCoverage, Columns::Every)
            && !moved(TableId::CountryLinks, Columns::These(&LINK_SLOT));
        if !fresh && quiet && self.finalized.is_empty() {
            self.last = Some(at);
            return Ok(());
        }
        let mut ended = Vec::new();
        let written: HashSet<u64> = if fresh {
            self.rebuild(hour, out, &mut ended)?;
            self.live.keys().copied().collect()
        } else {
            let (written, begun) = self.census(hour, &mut ended)?;
            self.counters(hour, &begun)?;
            written
        };
        let drain = drain(hour);
        let mut own = match drain {
            Some((from, _)) if !hour.first => self.finalize(hour, from, &mut ended)?,
            _ => Vec::new(),
        };
        for &combat in &written {
            if let Some(battle) = self.live.get(&combat) {
                out.set(TableId::LandBattles, u128::from(combat), battle.row());
            }
        }
        for end in ended {
            if !self.live.contains_key(&end.combat) {
                out.delete(TableId::LandBattles, u128::from(end.combat));
            }
            self.end_row(end, drain.is_some(), out)?;
        }
        own.sort_by_key(|outcome| (outcome.combat, outcome.end_hour, outcome.id));
        for outcomes in own.chunk_by(|a, b| (a.combat, a.end_hour) == (b.combat, b.end_hour)) {
            self.own_row(outcomes, out)?;
        }
        self.last = Some(at);
        Ok(())
    }

    fn prime(&mut self, hour: &Hour<'_>) {
        *self = Self::default();
        let ends = hour.table(TableId::LandBattleEnds);
        for slot in ends.rows() {
            let Some(key) = ends.key(slot) else { continue };
            self.living.push(key);
            if let Some((combat, End::FINALIZED)) = inputs::end(&ends, slot) {
                self.finalized.push(combat);
            }
        }
        let held = hour.table(TableId::LandBattles);
        let combats = hour.table(TableId::LandCombats);
        for slot in held.rows() {
            let Some(combat) = held.key(slot).map(|key| key as u64) else { continue };
            let Some(row) = inputs::held(&held, slot) else { continue };
            let mut battle = Battle::of(&row);
            if let Some(read) = combats.slot(u128::from(combat)).and_then(|slot| inputs::combat(&combats, slot, false).ok()) {
                battle.province = Some(read.province);
                battle.duration = Some(read.duration);
            }
            battle.raw = sides(hour, combat).ok().flatten();
            battle.countries = resolve(hour, battle.raw.as_ref());
            battle.manpower = manpower(hour, combat);
            self.live.insert(combat, battle);
        }
        self.last = Some(hour.hour);
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("LandBattles' state holds nothing"));
        }
        *self = Self::default();
        Ok(())
    }
}
