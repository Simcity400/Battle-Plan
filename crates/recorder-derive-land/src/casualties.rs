
use recorder_derive_framework::{
    forces::{exact, invalid, number, written},
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns},
    holders::{self, Holders, Identity},
};
use replay_columnar::{leaf::Writer as Bytes, manifest::Status, store::TableView};
use replay_data::rows::{CasualtyLifetimeRow, TableId, WarRelationRow};
use std::{
    collections::{HashMap, HashSet},
    io,
};

const RELATION_COLUMNS: [usize; 5] =
    columns::<WarRelationRow, 5>(["start_hour", "first_casualties", "second_casualties", "first_unknown_casualties", "second_unknown_casualties"]);

const READS: &[Read] = &[
    Read { table: TableId::WarRelations, columns: Columns::These(&RELATION_COLUMNS), watch: Watch::Changed, required: true },
    Read { table: TableId::Countries, columns: Columns::These(&holders::COUNTRY_SLOT), watch: Watch::Changed, required: true },
    Read { table: TableId::CountryLinks, columns: Columns::These(&holders::LINK_SLOT), watch: Watch::Changed, required: true },
];

const TABLES: [TableId; 2] = [TableId::CasualtyLifetimes, TableId::CasualtyTotals];

const UNRESOLVED: usize = 0;
const SAME: usize = 1;
const WIDE: usize = 2;
const TWICE: usize = 3;
const REASONS: [&str; 4] = [
    "Casualties: a war relation's country resolves to no identity",
    "Casualties: a war relation joins a country identity to itself",
    "Casualties: a war relation's country slot or id passes the 24 bits a lifetime's key holds",
    "Casualties: two war relations are one lifetime",
];

const DEALT: usize = 0;
const TAKEN: usize = 1;
const UNKNOWN: usize = 2;
const PARTIAL: usize = 3;

type Counters = [i64; 4];

fn lifetime_key(a: Identity, b: Identity, start: i32) -> u128 {
    u128::from(a.0) << 104 | u128::from(a.1) << 80 | u128::from(b.0) << 56 | u128::from(b.1) << 32 | u128::from(start as u32)
}

fn lifetime_of(key: u128) -> (Identity, Identity) {
    let field = |shift: u32| (key >> shift) as u32 & CasualtyLifetimeRow::FIELD;
    ((field(104), field(80)), (field(56), field(32)))
}

struct Relation {
    raw: u64,
    key: u128,
    counters: Counters,
}

fn relation(holders: &Holders, view: &TableView<'_>, slot: u32) -> io::Result<Result<Relation, usize>> {
    let table = TableId::WarRelations;
    let raw = view.key(slot).ok_or_else(|| invalid(format!("WarRelations slot {slot} holds no row")))? as u64;
    let cell = |at: usize| number(view, table, RELATION_COLUMNS[at], slot);
    let start = cell(0)? as i32;
    let read = [cell(1)?, cell(2)?, cell(3)?, cell(4)?];
    let (Some(first), Some(second)) = (holders.identity((raw >> 32) as u32), holders.identity(raw as u32)) else {
        return Ok(Err(UNRESOLVED));
    };
    if first == second {
        return Ok(Err(SAME));
    }
    if [first.0, first.1, second.0, second.1].iter().any(|&field| field > CasualtyLifetimeRow::FIELD) {
        return Ok(Err(WIDE));
    }
    let (a, b, counters) = if first < second { (first, second, read) } else { (second, first, [read[1], read[0], read[3], read[2]]) };
    Ok(Ok(Relation { raw, key: lifetime_key(a, b, start), counters }))
}

fn held(view: &TableView<'_>, key: u128) -> io::Result<Option<Counters>> {
    let Some(slot) = view.slot(key) else { return Ok(None) };
    let table = TableId::CasualtyLifetimes;
    let cell = |column: usize| number(view, table, column, slot);
    Ok(Some([cell(0)?, cell(1)?, cell(2)?, cell(3)?]))
}

fn lifetime_cells(counters: Counters) -> Vec<Value> {
    counters.into_iter().map(Value::I64).collect()
}

type Sums = [Option<i128>; 2];

#[derive(Clone, Copy, Debug)]
struct Agree {
    present: bool,
    all_zero: bool,
    delta: Option<i64>,
    valid: bool,
    new_max: i64,
}

impl Default for Agree {
    fn default() -> Self {
        Self { present: false, all_zero: true, delta: None, valid: true, new_max: 0 }
    }
}

impl Agree {
    fn relation(&mut self, value: i64, previous: Option<i64>) {
        self.present = true;
        self.all_zero &= value == 0;
        match previous {
            Some(previous) => match value.checked_sub(previous) {
                Some(delta) if delta >= 0 && self.delta.is_none_or(|seen| seen == delta) => self.delta = Some(delta),
                _ => self.valid = false,
            },
            None => {
                self.valid &= value >= 0;
                self.new_max = self.new_max.max(value);
            }
        }
    }

    fn unchanged(&mut self) {
        self.present = true;
        match self.delta {
            None => self.delta = Some(0),
            Some(0) => {}
            Some(_) => self.valid = false,
        }
    }

    fn total(&self, seen: bool, total: Option<i64>) -> Option<i64> {
        if !self.present {
            return total;
        }
        if !seen {
            return self.all_zero.then_some(0);
        }
        total.and_then(|total| self.delta.filter(|&delta| self.valid && self.new_max <= delta).and_then(|delta| total.checked_add(delta)))
    }
}

struct Plan {
    moved: Vec<Relation>,
    retired: Vec<u128>,
    gone: Vec<u64>,
}

#[derive(Default)]
struct Touch {
    agree: Agree,
    moved: u32,
    partial: bool,
}

#[derive(Default)]
pub struct Casualties {
    rebuild: bool,
    holders: Holders,
    by_raw: HashMap<u64, u128>,
    live: HashSet<u128>,
    degree: HashMap<Identity, u32>,
    sums: HashMap<Identity, Sums>,
}

fn add(sums: &mut HashMap<Identity, Sums>, key: u128, old: Counters, new: Counters) {
    let (a, b) = lifetime_of(key);
    let (taken_a, taken_b) = (i128::from(new[0]) - i128::from(old[0]), i128::from(new[1]) - i128::from(old[1]));
    for (identity, deltas) in [(a, [taken_b, taken_a]), (b, [taken_a, taken_b])] {
        let held = sums.entry(identity).or_insert([Some(0); 2]);
        for (sum, delta) in held.iter_mut().zip(deltas) {
            *sum = sum.map(|sum| sum + delta).filter(|&sum| i64::try_from(sum).is_ok());
        }
    }
}

impl Casualties {
    fn read_tables(&mut self, out: &Rows<'_>) -> io::Result<()> {
        self.by_raw.clear();
        self.live.clear();
        self.degree.clear();
        self.sums.clear();
        let view = out.table(TableId::CasualtyLifetimes);
        for slot in view.rows() {
            self.live.insert(view.key(slot).ok_or_else(|| invalid(format!("CasualtyLifetimes slot {slot} holds no row")))?);
        }
        let totals = out.table(TableId::CasualtyTotals);
        for slot in totals.rows() {
            let key = totals.key(slot).ok_or_else(|| invalid(format!("CasualtyTotals slot {slot} holds no row")))?;
            self.sums.insert(holders::of(key), [DEALT, TAKEN].map(|column| totals.number(column, slot).map(i128::from)));
        }
        Ok(())
    }

    fn plan_whole(&self, hour: &Hour<'_>) -> io::Result<Result<Plan, usize>> {
        let view = hour.table(TableId::WarRelations);
        let mut moved = Vec::new();
        let mut keys = HashSet::new();
        for slot in view.rows() {
            let relation = match relation(&self.holders, &view, slot)? {
                Ok(relation) => relation,
                Err(reason) => return Ok(Err(reason)),
            };
            if !keys.insert(relation.key) {
                return Ok(Err(TWICE));
            }
            moved.push(relation);
        }
        let retired = self.live.iter().filter(|key| !keys.contains(key)).copied().collect();
        Ok(Ok(Plan { moved, retired, gone: Vec::new() }))
    }

    fn plan_changes(&self, hour: &Hour<'_>) -> io::Result<Result<Plan, usize>> {
        let mut retired = HashSet::new();
        let mut gone = Vec::new();
        for raw in hour.deleted_keys(TableId::WarRelations) {
            let raw = raw as u64;
            if let Some(&key) = self.by_raw.get(&raw) {
                retired.insert(key);
                gone.push(raw);
            }
        }
        let view = hour.table(TableId::WarRelations);
        let mut moved = Vec::new();
        let mut keys = HashSet::new();
        for slot in written(hour, TableId::WarRelations, &RELATION_COLUMNS) {
            let relation = match relation(&self.holders, &view, slot)? {
                Ok(relation) => relation,
                Err(reason) => return Ok(Err(reason)),
            };
            if !keys.insert(relation.key) {
                return Ok(Err(TWICE));
            }
            if let Some(&old) = self.by_raw.get(&relation.raw)
                && old != relation.key
            {
                retired.insert(old);
            }
            moved.push(relation);
        }
        if moved.iter().any(|relation| self.live.contains(&relation.key) && self.by_raw.get(&relation.raw) != Some(&relation.key) && !retired.contains(&relation.key)) {
            return Ok(Err(TWICE));
        }
        Ok(Ok(Plan { moved, retired: retired.into_iter().collect(), gone }))
    }

    fn link(&mut self, key: u128, sign: i32) {
        let (a, b) = lifetime_of(key);
        for identity in [a, b] {
            let degree = self.degree.entry(identity).or_default();
            *degree = degree.saturating_add_signed(sign);
            if *degree == 0 {
                self.degree.remove(&identity);
            }
        }
    }

    fn apply(&mut self, plan: Plan, whole: bool, gap: bool, out: &mut Rows<'_>) -> io::Result<()> {
        let moving: HashSet<u128> = plan.moved.iter().map(|relation| relation.key).collect();
        let mut retired: Vec<u128> = plan.retired.into_iter().filter(|key| !moving.contains(key)).collect();
        retired.sort_unstable();
        retired.dedup();
        let mut touched: HashMap<Identity, Touch> = HashMap::new();
        for &key in &retired {
            if out.table(TableId::CasualtyLifetimes).slot(key).is_none() {
                continue;
            }
            out.delete(TableId::CasualtyLifetimes, key);
            if gap {
                let (a, b) = lifetime_of(key);
                touched.entry(a).or_default().partial = true;
                touched.entry(b).or_default().partial = true;
            }
        }
        for relation in &plan.moved {
            let old = held(&out.table(TableId::CasualtyLifetimes), relation.key)?;
            let spans = old.is_some() && !gap;
            let old = old.unwrap_or([0; 4]);
            let new = relation.counters;
            add(&mut self.sums, relation.key, old, new);
            let (a, b) = lifetime_of(relation.key);
            let touch = touched.entry(a).or_default();
            touch.agree.relation(new[2], spans.then_some(old[2]));
            touch.moved += 1;
            let touch = touched.entry(b).or_default();
            touch.agree.relation(new[3], spans.then_some(old[3]));
            touch.moved += 1;
            out.set(TableId::CasualtyLifetimes, relation.key, lifetime_cells(new));
        }
        if whole {
            self.by_raw = plan.moved.iter().map(|relation| (relation.raw, relation.key)).collect();
            self.live = moving;
            self.degree.clear();
            for key in self.live.clone() {
                self.link(key, 1);
            }
        } else {
            for raw in plan.gone {
                self.by_raw.remove(&raw);
            }
            for key in &retired {
                if self.live.remove(key) {
                    self.link(*key, -1);
                }
            }
            for relation in &plan.moved {
                self.by_raw.insert(relation.raw, relation.key);
                if self.live.insert(relation.key) {
                    self.link(relation.key, 1);
                }
            }
            for (identity, touch) in &mut touched {
                if touch.moved > 0 && self.degree.get(identity).copied().unwrap_or(0) > touch.moved {
                    touch.agree.unchanged();
                }
            }
        }
        let rows: Vec<(u128, Vec<Value>)> = {
            let totals = out.table(TableId::CasualtyTotals);
            if gap {
                for key in totals.rows().filter_map(|slot| totals.key(slot)) {
                    touched.entry(holders::of(key)).or_default();
                }
            }
            touched
                .into_iter()
                .map(|(identity, touch)| {
                    let key = holders::key(identity);
                    let slot = totals.slot(key);
                    let before = if gap { None } else { slot.and_then(|slot| totals.number(UNKNOWN, slot)) };
                    let partial = touch.partial || slot.and_then(|slot| totals.number(PARTIAL, slot)).is_some_and(|partial| partial != 0);
                    let [dealt, taken] = self.sums.get(&identity).copied().unwrap_or([Some(0); 2]).map(|sum| sum.map_or(Value::Unrecorded, exact));
                    let unknown = touch.agree.total(slot.is_some(), before).map_or(Value::Unrecorded, Value::I64);
                    (key, vec![dealt, taken, unknown, Value::U8(u8::from(partial))])
                })
                .collect()
        };
        for (key, cells) in rows {
            out.set(TableId::CasualtyTotals, key, cells);
        }
        Ok(())
    }
}

impl Writer for Casualties {
    fn name(&self) -> &'static str {
        "Casualties"
    }

    fn version(&self) -> u32 {
        2
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::CasualtyLifetimes), Output::State(TableId::CasualtyTotals)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn strings(&self) -> &'static [&'static str] {
        &REASONS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let gap = !hour.first && out.table(TableId::CasualtyTotals).status() != Status::Complete;
        let reread = hour.fresh || self.rebuild;
        let whole = reread || gap || Holders::moved(hour);
        if !whole && !hour.wrote(TableId::WarRelations, Columns::These(&RELATION_COLUMNS)) {
            return Ok(());
        }
        if reread {
            self.read_tables(out)?;
            self.rebuild = false;
        }
        if whole {
            self.holders.read(hour);
        }
        let plan = if whole { self.plan_whole(hour)? } else { self.plan_changes(hour)? };
        match plan {
            Ok(plan) => self.apply(plan, whole, gap, out),
            Err(reason) => {
                for table in TABLES {
                    out.status(table, Status::Unavailable { reason: hour.strings[reason] });
                }
                Ok(())
            }
        }
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("Casualties' state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
