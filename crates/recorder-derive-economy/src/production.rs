
use recorder_derive_framework::{
    forces::held,
    framework::{Columns, Hour, Output as Written, Read, Rows, Value, Watch, Writer, columns},
    producers::{Producers, SLOT},
};
use replay_columnar::{
    leaf::{Reader, Writer as Bytes},
    store::TableView,
};
use replay_data::rows::{OutputPurpose, ProductionCountryTotalRow as CountryTotal, ProductionCoverageRow as Coverage, ProductionTotalRow as Total, ProductionTypeTotalRow as TypeTotal, RocketProductionOutputRow, TableId};
use std::{
    collections::{HashMap, HashSet},
    io,
};

mod inputs;
#[cfg(test)]
mod tests;

use inputs::*;

const READS: &[Read] = &[
    Read { table: TableId::Countries, columns: Columns::These(&SLOT), watch: Watch::Changed, required: true },
    Read { table: TableId::CountryLinks, columns: Columns::These(&SLOT), watch: Watch::Changed, required: true },
    Read { table: TableId::ProductionLineOutput, columns: Columns::These(&OUTPUT), watch: Watch::Dropped, required: true },
    Read { table: TableId::RocketProductionOutput, columns: Columns::These(&ROCKET_COLUMNS), watch: Watch::Dropped, required: false },
    Read { table: TableId::VariantDefinitions, columns: Columns::These(&TOKEN), watch: Watch::Changed, required: false },
    Read { table: TableId::LexicalTokens, columns: Columns::These(&LITERAL), watch: Watch::Changed, required: false },
];

const TABLES: &[Written] = &[Written::State(TableId::ProductionTotals), Written::State(TableId::ProductionCountryTotals), Written::State(TableId::ProductionTypeTotals), Written::State(TableId::ProductionCoverage)];

const TOTAL: [usize; 7] = columns::<Total, 7>(["units", "inexact_rows", "spent", "spent_reason", "unproven", "equipment_token", "archetype"]);
const COUNTRY: [usize; 6] = columns::<CountryTotal, 6>(["units", "units_inexact", "spent", "units_without_refits", "units_without_refits_inexact", "spent_without_refits"]);
const TYPE: [usize; 7] = columns::<TypeTotal, 7>(["units", "units_inexact", "spent", "units_without_refits", "units_without_refits_inexact", "spent_without_refits", "archetype_spelling"]);
const COVERAGE: [usize; 4] = columns::<Coverage, 4>(["observation_since", "units_unknown_from", "spending_unknown_from", "spending_reason"]);

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

fn total_key(country: u32, variant: u64, purpose: u8) -> u128 {
    u128::from(country) << 72 | u128::from(variant) << 8 | u128::from(purpose)
}

fn total_parts(key: u128) -> (u32, u64, u8) {
    ((key >> 72) as u32, (key >> 8) as u64, key as u8)
}

fn type_key(country: u32, archetype: u32) -> u128 {
    u128::from(country) << 32 | u128::from(archetype)
}

fn coverage_key(country: u32, source: u8) -> u128 {
    u128::from(country) << 8 | u128::from(source)
}

fn add(a: i64, b: i64) -> io::Result<i64> {
    a.checked_add(b).ok_or_else(|| invalid("A production total leaves its exact integer range"))
}

fn sub(a: i64, b: i64) -> io::Result<i64> {
    a.checked_sub(b).ok_or_else(|| invalid("A production total leaves its exact integer range"))
}

#[derive(Clone, Copy, Debug, Default)]
struct Delta {
    units: i64,
    inexact: u32,
    spent: i64,
    reason: u8,
    unproven: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Held {
    units: i64,
    inexact: u32,
    spent: Option<i64>,
    reason: u8,
    unproven: Option<i64>,
    token: Option<u32>,
    archetype: u32,
}

impl Held {
    const NEW: Self = Self { units: 0, inexact: 0, spent: Some(0), reason: Total::KNOWN, unproven: Some(0), token: None, archetype: TypeTotal::NO_ARCHETYPE };

    fn read(view: &TableView<'_>, key: u128) -> Option<Self> {
        let slot = view.slot(key)?;
        Some(Self {
            units: view.number(TOTAL[0], slot)?,
            inexact: view.number(TOTAL[1], slot)? as u32,
            spent: view.number(TOTAL[2], slot),
            reason: view.number(TOTAL[3], slot)? as u8,
            unproven: view.number(TOTAL[4], slot),
            token: view.text(TOTAL[5], slot),
            archetype: view.number(TOTAL[6], slot)? as u32,
        })
    }

    fn apply(self, delta: Delta, token: Option<u32>, archetype: Option<u32>) -> io::Result<Self> {
        let reason = if self.reason != Total::KNOWN { self.reason } else { delta.reason };
        let known = reason == Total::KNOWN;
        Ok(Self {
            units: add(self.units, delta.units)?,
            inexact: self.inexact.saturating_add(delta.inexact),
            spent: match (known, self.spent) {
                (true, Some(spent)) => Some(add(spent, delta.spent)?),
                _ => None,
            },
            reason,
            unproven: match (known, self.unproven) {
                (true, Some(unproven)) => Some(add(unproven, delta.unproven)?),
                _ => None,
            },
            token: token.or(self.token),
            archetype: archetype.unwrap_or(self.archetype),
        })
    }

    fn cells(&self) -> Vec<Value> {
        vec![
            Value::I64(self.units),
            Value::U32(self.inexact),
            self.spent.map_or(Value::Unrecorded, Value::I64),
            Value::U8(self.reason),
            self.unproven.map_or(Value::Unrecorded, Value::I64),
            self.token.map_or(Value::Unrecorded, Value::Bytes),
            Value::U32(self.archetype),
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Sum {
    units: i64,
    inexact: u32,
    spent: Option<i64>,
}

impl Sum {
    const ZERO: Self = Self { units: 0, inexact: 0, spent: Some(0) };

    fn take(&mut self, old: &Held, new: &Held) -> io::Result<()> {
        self.units = add(self.units, sub(new.units, old.units)?)?;
        self.inexact = (self.inexact + u32::from(new.inexact > 0)).saturating_sub(u32::from(old.inexact > 0));
        self.spent = match (self.spent, old.spent, new.spent) {
            (Some(sum), Some(old), Some(new)) => Some(add(sum, sub(new, old)?)?),
            _ => None,
        };
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Country {
    all: Sum,
    without_refits: Sum,
}

impl Country {
    const ZERO: Self = Self { all: Sum::ZERO, without_refits: Sum::ZERO };

    fn read(view: &TableView<'_>, key: u128, columns: &[usize]) -> Option<Self> {
        let slot = view.slot(key)?;
        Some(Self {
            all: Sum { units: view.number(columns[0], slot)?, inexact: view.number(columns[1], slot)? as u32, spent: view.number(columns[2], slot) },
            without_refits: Sum { units: view.number(columns[3], slot)?, inexact: view.number(columns[4], slot)? as u32, spent: view.number(columns[5], slot) },
        })
    }

    fn take(&mut self, old: &Held, new: &Held, purpose: u8) -> io::Result<()> {
        self.all.take(old, new)?;
        if purpose != OutputPurpose::Refit.raw() {
            self.without_refits.take(old, new)?;
        }
        Ok(())
    }

    fn cells(&self) -> Vec<Value> {
        vec![
            Value::I64(self.all.units),
            Value::U32(self.all.inexact),
            self.all.spent.map_or(Value::Unrecorded, Value::I64),
            Value::I64(self.without_refits.units),
            Value::U32(self.without_refits.inexact),
            self.without_refits.spent.map_or(Value::Unrecorded, Value::I64),
        ]
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Covered {
    since: Option<i32>,
    units: Option<i32>,
    spending: Option<(i32, u8)>,
}

impl Covered {
    fn read(view: &TableView<'_>, key: u128) -> Option<Self> {
        let slot = view.slot(key)?;
        let hour = |column: usize| view.number(column, slot).map(|hour| hour as i32);
        Some(Self {
            since: hour(COVERAGE[0]),
            units: hour(COVERAGE[1]),
            spending: hour(COVERAGE[2]).map(|from| (from, view.number(COVERAGE[3], slot).unwrap_or(0) as u8)),
        })
    }

    fn units(&mut self, hour: i32) {
        self.units = Some(self.units.map_or(hour, |from| from.min(hour)));
    }

    fn spending(&mut self, hour: i32, reason: u8) {
        if self.spending.is_none_or(|(from, _)| hour < from) {
            self.spending = Some((hour, reason));
        }
    }

    fn cells(&self) -> Vec<Value> {
        vec![
            Value::OptionalI32(self.since),
            Value::OptionalI32(self.units),
            Value::OptionalI32(self.spending.map(|(from, _)| from)),
            Value::U8(self.spending.map_or(Total::KNOWN, |(_, reason)| reason)),
        ]
    }
}

fn consistent(previous: &RocketProductionOutputRow, row: &RocketProductionOutputRow) -> (bool, bool) {
    let spending = row.ticks >= previous.ticks
        && row.inexact_ticks >= previous.inexact_ticks
        && row.unknown_design_ticks >= previous.unknown_design_ticks
        && row.unknown_country_ticks >= previous.unknown_country_ticks
        && (!row.exact || previous.exact && (row.ticks != previous.ticks || (row.spent_raw == previous.spent_raw && row.instant_raw == previous.instant_raw)));
    let units = row.deliveries >= previous.deliveries
        && row.inexact_deliveries >= previous.inexact_deliveries
        && (!row.units_exact || previous.units_exact && (row.deliveries != previous.deliveries || row.units == previous.units));
    (spending, units)
}

#[derive(Default)]
pub struct Production {
    rebuild: bool,
    producers: Producers,
    pins: HashMap<u128, Option<u32>>,
    untold: HashSet<u128>,
}

impl Production {
    fn moved(hour: &Hour<'_>, whole: bool) -> io::Result<Vec<(u128, Option<Output>, Option<Output>)>> {
        let table = TableId::ProductionLineOutput;
        let view = hour.table(table);
        let unreadable = || invalid("A ProductionLineOutput row's cells are not recorded");
        let mut moved = Vec::new();
        if whole {
            for slot in view.rows() {
                let key = view.key(slot).ok_or_else(unreadable)?;
                moved.push((key, Some(output_now(&view, slot).ok_or_else(unreadable)?), None));
            }
            return Ok(moved);
        }
        let inserted: HashSet<u32> = hour.changes(table).inserted.iter().copied().collect();
        let written = written(hour, table);
        for slot in hour.written_rows(table, Columns::These(&OUTPUT)) {
            let key = view.key(slot).ok_or_else(unreadable)?;
            let now = output_now(&view, slot).ok_or_else(unreadable)?;
            let before = match inserted.contains(&slot) {
                true => None,
                false => Some(output_before(&view, slot, written.get(&slot).map_or(&[][..], Vec::as_slice)).ok_or_else(unreadable)?),
            };
            moved.push((key, Some(now), before));
        }
        for (key, cells) in hour.dropped(table) {
            moved.push((*key, None, Some(output_dropped(cells).ok_or_else(unreadable)?)));
        }
        Ok(moved)
    }

    fn lines(&mut self, hour: &Hour<'_>, out: &mut Rows, reread: bool) -> io::Result<()> {
        let at = hour.hour;
        let run_key = coverage_key(Coverage::RUN, Coverage::LINES);
        let held_run = Covered::read(&out.table(TableId::ProductionCoverage), run_key);
        let mut run = held_run.unwrap_or_default();
        let since = |moved: &[(u128, Option<Output>, Option<Output>)]| moved.iter().filter_map(|(_, now, _)| now.map(|row| row.since)).min();
        let mut restart = hour.first || held_run.is_none();
        let mut moved = Self::moved(hour, restart)?;
        match run.since {
            None => run.since = since(&moved),
            Some(held) if moved.iter().any(|(_, now, _)| now.is_some_and(|row| row.since != held)) => {
                restart = true;
                moved = Self::moved(hour, true)?;
                run = Covered { since: since(&moved), ..Covered::default() };
            }
            Some(_) => {}
        }
        if restart {
            self.pins.clear();
            self.untold.clear();
        }
        let mut left: HashMap<u128, Option<u32>> = HashMap::new();
        for (key, now, _) in &moved {
            if now.is_none()
                && let Some(pin) = self.pins.remove(key).or_else(|| self.untold.remove(key).then_some(None))
            {
                left.insert(*key, pin);
            }
        }
        if hour.fresh && !hour.first {
            run.units(at);
            run.spending(at, Total::MISSED_PUBLICATION);
            let view = hour.table(TableId::ProductionLineOutput);
            self.pins.retain(|key, _| view.slot(*key).is_some());
            self.untold.retain(|key| view.slot(*key).is_some());
        }
        let mut deltas: HashMap<u128, Delta> = HashMap::new();
        for (key, now, before) in moved {
            match (now, before) {
                (Some(now), before) => {
                    let (units, exact, spent) = before.map_or((0, true, Spent::Value(0)), |row| (row.units, row.exact, row.spent));
                    let moved_units = sub(now.units, units)?;
                    let (moved_spent, reason) = match (now.spent, spent) {
                        (Spent::Value(now), Spent::Value(before)) => (sub(now, before)?, Total::KNOWN),
                        (Spent::Value(_), _) => (0, Total::KNOWN),
                        (Spent::Refused, _) => (0, Total::SPENDING_UNAVAILABLE),
                        (Spent::NotRecorded, _) => (0, Total::NOT_RECORDED),
                    };
                    let producer = match self.pins.get(&key) {
                        Some(&pin) => pin,
                        None => {
                            let told = self.producers.of(now.country);
                            if told.is_some() || now.units != 0 || !now.exact {
                                self.untold.remove(&key);
                                self.pins.insert(key, told);
                            } else {
                                self.untold.insert(key);
                            }
                            told
                        }
                    };
                    match producer {
                        Some(country) => {
                            let delta = deltas.entry(total_key(country, now.variant, now.purpose)).or_default();
                            delta.units = add(delta.units, moved_units)?;
                            delta.inexact += u32::from(exact && !now.exact);
                            delta.spent = add(delta.spent, moved_spent)?;
                            if delta.reason == Total::KNOWN {
                                delta.reason = reason;
                            }
                            if now.purpose == OutputPurpose::Unknown.raw() {
                                delta.unproven = add(delta.unproven, moved_spent.checked_abs().ok_or_else(|| invalid("Applied IC leaves its exact integer range"))?)?;
                            }
                        }
                        None => {
                            if moved_units != 0 || !now.exact {
                                run.units(at);
                                run.spending(at, Total::MISSED_PUBLICATION);
                            }
                            if moved_spent != 0 || reason != Total::KNOWN {
                                run.spending(at, Total::SPENDING_UNAVAILABLE);
                            }
                        }
                    }
                }
                (None, Some(gone)) if !gone.ended => match left.get(&key).copied().unwrap_or_else(|| self.producers.of(gone.country)) {
                    Some(country) => {
                        let delta = deltas.entry(total_key(country, gone.variant, gone.purpose)).or_default();
                        delta.inexact += u32::from(gone.exact);
                        delta.reason = Total::LOST_OUTPUT;
                    }
                    None => {
                        run.units(at);
                        run.spending(at, Total::MISSED_PUBLICATION);
                    }
                },
                _ => {}
            }
        }
        if reread && !self.untold.is_empty() {
            let view = hour.table(TableId::ProductionLineOutput);
            let told: Vec<(u128, u32)> = self
                .untold
                .iter()
                .filter_map(|&key| Some((key, self.producers.of(output_now(&view, view.slot(key)?)?.country)?)))
                .collect();
            for (key, producer) in told {
                self.untold.remove(&key);
                self.pins.insert(key, Some(producer));
            }
        }
        if held_run != Some(run) {
            out.set(TableId::ProductionCoverage, run_key, run.cells());
        }
        self.totals(hour, out, deltas, restart, reread)
    }

    fn totals(&self, hour: &Hour<'_>, out: &mut Rows<'_>, deltas: HashMap<u128, Delta>, restart: bool, reread: bool) -> io::Result<()> {
        if restart {
            for table in [TableId::ProductionTotals, TableId::ProductionTypeTotals] {
                for key in held(out, table) {
                    out.delete(table, key);
                }
            }
        }
        let country_row = |out: &Rows<'_>, country: u32| Country::read(&out.table(TableId::ProductionCountryTotals), u128::from(country), &COUNTRY);
        let type_row = |out: &Rows<'_>, key: u128| Country::read(&out.table(TableId::ProductionTypeTotals), key, &TYPE);
        let mut rows: HashMap<u32, Country> = HashMap::new();
        let mut kinds: HashMap<u128, Country> = HashMap::new();
        let mut keys: Vec<(u128, Delta)> = deltas.into_iter().collect();
        keys.sort_unstable_by_key(|(key, _)| *key);
        for (key, delta) in keys {
            let (country, variant, purpose) = total_parts(key);
            let held = if restart { None } else { Held::read(&out.table(TableId::ProductionTotals), key) };
            let old = held.unwrap_or(Held::NEW);
            let definition = definition(hour, variant);
            let new = old.apply(delta, definition.and_then(|(token, _)| spelling(hour, token)), definition.map(|(_, archetype)| archetype))?;
            out.set(TableId::ProductionTotals, key, new.cells());
            let row = rows.entry(country).or_insert_with(|| (if restart { None } else { country_row(out, country) }).unwrap_or(Country::ZERO));
            row.take(&old, &new, purpose)?;
            for owner in [country, TypeTotal::WORLD] {
                let parts = match held.is_none() || old.archetype == new.archetype {
                    true => vec![(new.archetype, old, new)],
                    false => vec![(old.archetype, old, Held::NEW), (new.archetype, Held::NEW, new)],
                };
                for (archetype, from, to) in parts {
                    let key = type_key(owner, archetype);
                    let kind = kinds.entry(key).or_insert_with(|| (if restart { None } else { type_row(out, key) }).unwrap_or(Country::ZERO));
                    kind.take(&from, &to, purpose)?;
                }
            }
        }
        for (key, row) in kinds {
            if restart || type_row(out, key) != Some(row) {
                let types = out.table(TableId::ProductionTypeTotals);
                let spelled = match key as u32 {
                    TypeTotal::NO_ARCHETYPE => None,
                    archetype => spelling(hour, archetype).or_else(|| types.slot(key).and_then(|slot| types.text(TYPE[6], slot))),
                };
                let mut cells = row.cells();
                cells.push(spelled.map_or(Value::Unrecorded, Value::Bytes));
                out.set(TableId::ProductionTypeTotals, key, cells);
            }
        }
        if restart || reread {
            for (_, country) in self.producers.holders() {
                rows.entry(country).or_insert_with(|| (if restart { None } else { country_row(out, country) }).unwrap_or(Country::ZERO));
            }
            for key in held(out, TableId::ProductionCountryTotals) {
                let country = key as u32;
                if rows.contains_key(&country) || self.producers.live(country) {
                    continue;
                }
                if restart || country_row(out, country) == Some(Country::ZERO) {
                    out.delete(TableId::ProductionCountryTotals, key);
                }
            }
        }
        for (country, row) in rows {
            if restart || country_row(out, country) != Some(row) {
                out.set(TableId::ProductionCountryTotals, u128::from(country), row.cells());
            }
        }
        Ok(())
    }

    fn rocket(&self, hour: &Hour<'_>, out: &mut Rows<'_>) -> io::Result<()> {
        let table = TableId::RocketProductionOutput;
        if !hour.complete(table) {
            return Ok(());
        }
        let view = hour.table(table);
        let unreadable = || invalid("A RocketProductionOutput row's cells are not recorded");
        let Some(world) = view.slot(0).map(|slot| rocket_now(&view, slot).ok_or_else(unreadable)).transpose()? else {
            return Ok(());
        };
        let at = hour.hour;
        let run_key = coverage_key(Coverage::RUN, Coverage::ROCKET);
        let held_run = Covered::read(&out.table(TableId::ProductionCoverage), run_key);
        let restart = held_run.is_none_or(|run| run.since != Some(world.since_hour));
        let mut run = match restart {
            true => Covered { since: Some(world.since_hour), ..Covered::default() },
            false => held_run.unwrap_or_default(),
        };
        let mut rows: Vec<(Option<RocketProductionOutputRow>, Option<RocketProductionOutputRow>)> = Vec::new();
        if restart {
            for key in held(out, TableId::ProductionCoverage) {
                if key as u8 == Coverage::ROCKET && key != run_key {
                    out.delete(TableId::ProductionCoverage, key);
                }
            }
            for slot in view.rows() {
                rows.push((Some(rocket_now(&view, slot).ok_or_else(unreadable)?), None));
            }
        } else {
            let inserted: HashSet<u32> = hour.changes(table).inserted.iter().copied().collect();
            let written = written(hour, table);
            for slot in hour.written_rows(table, Columns::These(&ROCKET_COLUMNS)) {
                let now = rocket_now(&view, slot).ok_or_else(unreadable)?;
                let before = match inserted.contains(&slot) {
                    true => None,
                    false => Some(rocket_before(&view, slot, written.get(&slot).map_or(&[][..], Vec::as_slice)).ok_or_else(unreadable)?),
                };
                rows.push((Some(now), before));
            }
            for (key, cells) in hour.dropped(table) {
                rows.push((None, Some(rocket_dropped(*key, cells).ok_or_else(unreadable)?)));
            }
        }
        let covered_row = |out: &Rows<'_>, country: u32| Covered::read(&out.table(TableId::ProductionCoverage), coverage_key(country, Coverage::ROCKET));
        let mut owners: HashMap<u32, Covered> = HashMap::new();
        for (now, before) in rows {
            let row = now.as_ref().or(before.as_ref()).expect("a row now or before");
            let owner = match (row.country, row.variant) {
                (0, 0) => Some(Coverage::RUN),
                (0, _) => None,
                (raw, _) => self.producers.of(raw),
            };
            let covered = match owner {
                Some(Coverage::RUN) | None => &mut run,
                Some(country) => owners.entry(country).or_insert_with(|| covered_row(out, country).unwrap_or_default()),
            };
            let Some(now) = now else {
                covered.units(at);
                covered.spending(at, Total::LOST_OUTPUT);
                continue;
            };
            if let Some(before) = &before {
                let (spending, units) = consistent(before, &now);
                if !spending {
                    covered.spending(at, Total::ROCKET_INPUT_INEXACT);
                }
                if !units {
                    covered.units(at);
                }
            }
            match owner {
                Some(Coverage::RUN) => {
                    if !now.exact {
                        run.spending(at, Total::ROCKET_OWNER_UNREADABLE);
                    }
                    if !now.units_exact {
                        run.units(at);
                    }
                }
                None => {
                    if now.ticks != 0 || now.spent_raw != 0 || !now.exact {
                        run.spending(at, Total::ROCKET_OWNER_UNREADABLE);
                    }
                    if now.deliveries != 0 || now.units != 0 || !now.units_exact {
                        run.units(at);
                    }
                }
                Some(_) => {}
            }
        }
        if restart || held_run != Some(run) {
            out.set(TableId::ProductionCoverage, run_key, run.cells());
        }
        for (country, covered) in owners {
            if covered != Covered::default() && covered_row(out, country) != Some(covered) {
                out.set(TableId::ProductionCoverage, coverage_key(country, Coverage::ROCKET), covered.cells());
            }
        }
        Ok(())
    }
}

impl Writer for Production {
    fn name(&self) -> &'static str {
        "Production"
    }

    fn version(&self) -> u32 {
        3
    }

    fn tables(&self) -> &'static [Written] {
        TABLES
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let reread = hour.fresh || self.rebuild || Producers::moved(hour);
        let lines = hour.fresh || reread || hour.changed(TableId::ProductionLineOutput);
        let rocket = hour.fresh || hour.changed(TableId::RocketProductionOutput);
        if !lines && !rocket {
            return Ok(());
        }
        if reread {
            self.producers.read(hour);
            self.rebuild = false;
        }
        if lines {
            self.lines(hour, out, reread)?;
        }
        if rocket {
            self.rocket(hour, out)?;
        }
        Ok(())
    }

    fn save(&self, blob: &mut Bytes) -> io::Result<()> {
        let mut pins: Vec<(u128, Option<u32>)> = self.pins.iter().map(|(&key, &pin)| (key, pin)).collect();
        pins.sort_unstable();
        blob.varint(pins.len() as u64)?;
        for (key, pin) in pins {
            blob.raw(&key.to_le_bytes())?;
            blob.varint(pin.map_or(0, |producer| u64::from(producer) + 1))?;
        }
        let mut untold: Vec<u128> = self.untold.iter().copied().collect();
        untold.sort_unstable();
        blob.varint(untold.len() as u64)?;
        for key in untold {
            blob.raw(&key.to_le_bytes())?;
        }
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        fn key(reader: &mut Reader<'_>) -> io::Result<u128> {
            Ok(u128::from_le_bytes(reader.raw(16)?.try_into().map_err(|_| invalid("A short row key"))?))
        }
        *self = Self { rebuild: true, ..Self::default() };
        let mut reader = Reader::new(blob);
        let count = reader.count()?;
        if count > blob.len() / 17 {
            return Err(invalid("Production's state counts more rows than it holds"));
        }
        for _ in 0..count {
            let row = key(&mut reader)?;
            let pin = match reader.varint()? {
                0 => None,
                held => Some(u32::try_from(held - 1).map_err(|_| invalid("A producer id past 32 bits"))?),
            };
            self.pins.insert(row, pin);
        }
        let count = reader.count()?;
        if count > blob.len() / 16 {
            return Err(invalid("Production's state counts more rows than it holds"));
        }
        for _ in 0..count {
            self.untold.insert(key(&mut reader)?);
        }
        reader.finish()
    }
}
