
use recorder_derive_framework::{
    forces::held,
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns},
};
use replay_columnar::{leaf::Writer as Bytes, store::TableView};
use replay_data::rows::{
    ProducedValueRow as Row, ProductionCountryTotalRow, ProductionCoverageRow as Coverage, ProductionTotalRow as Total, ProductionTypeTotalRow as TypeTotal,
    RocketProductionOutputRow, TableId, VariantDefinitionRow,
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
};

#[cfg(test)]
mod tests;

const TOTALS: [usize; 6] = columns::<ProductionCountryTotalRow, 6>(["units", "units_inexact", "spent", "units_without_refits", "units_without_refits_inexact", "spent_without_refits"]);
const TYPES: [usize; 6] = columns::<TypeTotal, 6>(["units", "units_inexact", "spent", "units_without_refits", "units_without_refits_inexact", "spent_without_refits"]);
const COVERAGE: [usize; 4] = columns::<Coverage, 4>(["observation_since", "units_unknown_from", "spending_unknown_from", "spending_reason"]);
const ROCKET: [usize; 3] = columns::<RocketProductionOutputRow, 3>(["units", "units_exact", "spent_raw"]);
const ARCHETYPE: [usize; 1] = columns::<VariantDefinitionRow, 1>(["archetype_token"]);

const READS: &[Read] = &[
    Read { table: TableId::ProductionCountryTotals, columns: Columns::Every, watch: Watch::Changed, required: true },
    Read { table: TableId::ProductionTypeTotals, columns: Columns::These(&TYPES), watch: Watch::Changed, required: true },
    Read { table: TableId::ProductionCoverage, columns: Columns::Every, watch: Watch::Changed, required: true },
    Read { table: TableId::RocketProductionOutput, columns: Columns::These(&ROCKET), watch: Watch::EveryRead, required: false },
    Read { table: TableId::VariantDefinitions, columns: Columns::These(&ARCHETYPE), watch: Watch::Changed, required: false },
    Read { table: TableId::ProductionLineOutput, columns: Columns::These(&[]), watch: Watch::EveryRead, required: false },
];

const TABLES: &[Output] = &[Output::State(TableId::ProducedValues)];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Amount {
    Known(i128),
    Unknown(u8),
}

impl Amount {
    pub fn plus(self, other: Self) -> Self {
        match (self, other) {
            (Self::Known(a), Self::Known(b)) => a.checked_add(b).map_or(Self::Unknown(Row::OUT_OF_RANGE), Self::Known),
            (Self::Unknown(reason), _) | (_, Self::Unknown(reason)) => Self::Unknown(reason),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Count {
    Units,
    UnitsWithoutRefits,
    Ic,
    IcWithoutRefits,
}

impl Count {
    pub const ALL: [Self; 4] = [Self::Units, Self::UnitsWithoutRefits, Self::Ic, Self::IcWithoutRefits];

    fn units(self) -> bool {
        matches!(self, Self::Units | Self::UnitsWithoutRefits)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Totals {
    pub units: i64,
    pub units_inexact: i64,
    pub spent: Option<i64>,
    pub units_without_refits: i64,
    pub units_without_refits_inexact: i64,
    pub spent_without_refits: Option<i64>,
}

impl Totals {
    pub fn of(number: impl Fn(usize) -> Option<i64>) -> Self {
        let (units, units_without_refits) = (number(0), number(3));
        Self {
            units: units.unwrap_or(0),
            units_inexact: number(1).unwrap_or(1) + i64::from(units.is_none()),
            spent: number(2),
            units_without_refits: units_without_refits.unwrap_or(0),
            units_without_refits_inexact: number(4).unwrap_or(1) + i64::from(units_without_refits.is_none()),
            spent_without_refits: number(5),
        }
    }

    pub fn amount(&self, count: Count) -> Amount {
        let units = |value: i64, inexact: i64| if inexact > 0 { Amount::Unknown(Row::UNITS_INEXACT) } else { Amount::Known(i128::from(value)) };
        let spent = |value: Option<i64>| value.map_or(Amount::Unknown(Row::SPENDING_UNAVAILABLE), |value| Amount::Known(i128::from(value)));
        match count {
            Count::Units => units(self.units, self.units_inexact),
            Count::UnitsWithoutRefits => units(self.units_without_refits, self.units_without_refits_inexact),
            Count::Ic => spent(self.spent),
            Count::IcWithoutRefits => spent(self.spent_without_refits),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RocketRow {
    pub units: i64,
    pub units_exact: bool,
    pub spent: i64,
}

impl RocketRow {
    fn amount(&self, count: Count) -> Amount {
        match count.units() {
            true if !self.units_exact => Amount::Unknown(Row::ROCKET_INPUT_INEXACT),
            true => Amount::Known(i128::from(self.units)),
            false => Amount::Known(i128::from(self.spent)),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Covered {
    pub observation: Option<i32>,
    pub units_unknown_from: Option<i32>,
    pub spending_unknown_from: Option<i32>,
    pub reason: u8,
}

impl Covered {
    fn unknown(&self, count: Count, hour: i32) -> Option<u8> {
        let from = if count.units() { self.units_unknown_from } else { self.spending_unknown_from }?;
        (from <= hour).then(|| reason(self.reason))
    }
}

pub fn reason(code: u8) -> u8 {
    match code {
        Total::NOT_RECORDED => Row::NOT_RECORDED,
        Total::LOST_OUTPUT => Row::LOST_OUTPUT,
        Total::MISSED_PUBLICATION => Row::MISSED_PUBLICATION,
        Total::ROCKET_INPUT_INEXACT => Row::ROCKET_INPUT_INEXACT,
        Total::ROCKET_OWNER_UNREADABLE => Row::ROCKET_OWNER_UNREADABLE,
        _ => Row::SPENDING_UNAVAILABLE,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Whole,
    Archetype(u32),
    Rest,
}

impl Part {
    fn archetype(self) -> u32 {
        match self {
            Self::Whole => Row::WHOLE,
            Self::Archetype(token) => token,
            Self::Rest => Row::REST,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Producer {
    pub country: Totals,
    pub types: BTreeMap<u32, Totals>,
    pub rocket: Vec<(u32, RocketRow)>,
    pub own_rocket: Option<Covered>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Run {
    pub lines: Option<Covered>,
    pub rocket: Option<Covered>,
    pub rocket_world: bool,
}

fn covered(rows: &[Option<Covered>], count: Count, hour: i32) -> Option<u8> {
    rows.iter().flatten().find_map(|row| row.unknown(count, hour))
}

pub fn total(producer: &Producer, run: &Run, part: Part, count: Count, hour: i32) -> Amount {
    let untyped = producer.types.get(&TypeTotal::NO_ARCHETYPE).map(|row| row.amount(count));
    let vetoed = untyped.is_some_and(|amount| amount != Amount::Known(0));
    let lines = match covered(&[run.lines], count, hour) {
        Some(why) => Amount::Unknown(why),
        None => match part {
            Part::Whole => producer.country.amount(count),
            Part::Archetype(TypeTotal::NO_ARCHETYPE) => untyped.unwrap_or(Amount::Known(0)),
            _ if vetoed => Amount::Unknown(Row::TYPE_NOT_RECORDED),
            Part::Archetype(token) => producer.types.get(&token).map_or(Amount::Known(0), |row| row.amount(count)),
            Part::Rest => Amount::Known(0),
        },
    };
    lines.plus(rocket(producer, run, part, count, hour))
}

fn rocket(producer: &Producer, run: &Run, part: Part, count: Count, hour: i32) -> Amount {
    if !run.rocket_world {
        return Amount::Unknown(Row::ROCKET_TABLE_UNAVAILABLE);
    }
    if let Some(why) = covered(&[producer.own_rocket, run.rocket], count, hour) {
        return Amount::Unknown(why);
    }
    let mut sum = Amount::Known(0);
    for (token, row) in &producer.rocket {
        let amount = row.amount(count);
        let counted = match part {
            Part::Whole => Some(true),
            Part::Archetype(asked) if asked == *token => Some(true),
            _ if *token == TypeTotal::NO_ARCHETYPE => None,
            _ => Some(false),
        };
        match counted {
            Some(true) => sum = sum.plus(amount),
            Some(false) => {}
            None if amount == Amount::Known(0) => {}
            None => return Amount::Unknown(Row::TYPE_NOT_RECORDED),
        }
    }
    sum
}

fn cells_of(amount: Amount) -> [Value; 2] {
    match amount {
        Amount::Known(value) => match i64::try_from(value) {
            Ok(value) => [Value::I64(value), Value::U8(Row::KNOWN)],
            Err(_) => [Value::Unrecorded, Value::U8(Row::OUT_OF_RANGE)],
        },
        Amount::Unknown(reason) => [Value::Unrecorded, Value::U8(reason)],
    }
}

pub fn row(producer: &Producer, run: &Run, part: Part, hour: i32) -> Vec<Value> {
    let mut cells: Vec<Value> = Count::ALL.iter().flat_map(|&count| cells_of(total(producer, run, part, count, hour))).collect();
    cells.extend([Value::OptionalI32(None), Value::OptionalI32(None)]);
    cells
}

pub fn parts(producer: &Producer) -> BTreeSet<u32> {
    let mut out: BTreeSet<u32> = producer.types.keys().copied().collect();
    out.extend(producer.rocket.iter().map(|(token, _)| *token));
    out
}

fn covered_row(view: &TableView<'_>, key: u128) -> Option<Covered> {
    let slot = view.slot(key)?;
    let hour = |column: usize| view.number(column, slot).and_then(|hour| i32::try_from(hour).ok());
    Some(Covered {
        observation: hour(COVERAGE[0]),
        units_unknown_from: hour(COVERAGE[1]),
        spending_unknown_from: hour(COVERAGE[2]),
        reason: view.number(COVERAGE[3], slot).map_or(Total::KNOWN, |code| code as u8),
    })
}

fn coverage_key(country: u32, source: u8) -> u128 {
    u128::from(country) << 8 | u128::from(source)
}

fn later(a: Option<i32>, b: Option<i32>) -> Option<i32> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (a, b) => a.or(b),
    }
}

#[derive(Default)]
pub struct ProducedValues {
    rebuild: bool,
    rocket_world: Option<bool>,
}

impl ProducedValues {
    fn run(hour: &Hour<'_>) -> Run {
        let coverage = hour.table(TableId::ProductionCoverage);
        let rocket = TableId::RocketProductionOutput;
        Run {
            lines: covered_row(&coverage, coverage_key(Coverage::RUN, Coverage::LINES)),
            rocket: covered_row(&coverage, coverage_key(Coverage::RUN, Coverage::ROCKET)),
            rocket_world: hour.complete(rocket) && hour.table(rocket).slot(0).is_some(),
        }
    }

    fn clock(hour: &Hour<'_>, out: &mut Rows<'_>, run: &Run) {
        let key = Row::clock();
        let mine = out.table(TableId::ProducedValues);
        let held_as_of = mine.slot(key).and_then(|slot| mine.number(9, slot)).and_then(|hour| i32::try_from(hour).ok());
        let lines = hour.table(TableId::ProductionLineOutput);
        let read = match lines.sampled_at() {
            Some(sampled) => sampled == hour.hour,
            None => hour.deleted_keys(TableId::ProductionTypeTotals).any(|key| (key >> 32) as u32 == TypeTotal::WORLD)
                || hour.written_rows(TableId::ProductionTypeTotals, Columns::These(&TYPES)).into_iter().any(|slot| {
                    hour.table(TableId::ProductionTypeTotals).key(slot).is_some_and(|key| (key >> 32) as u32 == TypeTotal::WORLD)
                }),
        };
        let as_of = if read { Some(hour.hour) } else { held_as_of };
        let since = later(run.lines.and_then(|row| row.observation), run.rocket.and_then(|row| row.observation));
        let mut cells: Vec<Value> = (0..4).flat_map(|_| [Value::Unrecorded, Value::U8(Row::KNOWN)]).collect();
        cells.extend([Value::OptionalI32(since), Value::OptionalI32(as_of)]);
        out.set(TableId::ProducedValues, key, cells);
    }

    fn moved(hour: &Hour<'_>) -> Option<BTreeSet<u32>> {
        let coverage = TableId::ProductionCoverage;
        let run_keys = [coverage_key(Coverage::RUN, Coverage::LINES), coverage_key(Coverage::RUN, Coverage::ROCKET)];
        let coverage_view = hour.table(coverage);
        let mut owners = BTreeSet::new();
        for slot in hour.written_rows(coverage, Columns::Every) {
            let key = coverage_view.key(slot)?;
            if run_keys.contains(&key) {
                return None;
            }
            owners.insert((key >> 8) as u32);
        }
        for key in hour.deleted_keys(coverage) {
            if run_keys.contains(&key) {
                return None;
            }
            owners.insert((key >> 8) as u32);
        }
        if hour.changed(TableId::VariantDefinitions) {
            return None;
        }
        let shift = [(TableId::ProductionCountryTotals, 0), (TableId::ProductionTypeTotals, 32), (TableId::RocketProductionOutput, 64)];
        for (table, by) in shift {
            let view = hour.table(table);
            let keys = hour.written_rows(table, Columns::Every).into_iter().filter_map(|slot| view.key(slot)).chain(hour.deleted_keys(table));
            owners.extend(keys.map(|key| (key >> by) as u32));
        }
        owners.remove(&TypeTotal::WORLD);
        owners.remove(&0);
        Some(owners)
    }

    fn producers(hour: &Hour<'_>, owners: Option<&BTreeSet<u32>>) -> HashMap<u32, Producer> {
        let wanted = |owner: u32| owners.is_none_or(|owners| owners.contains(&owner));
        let mut out: HashMap<u32, Producer> = HashMap::new();
        let countries = hour.table(TableId::ProductionCountryTotals);
        for slot in countries.rows() {
            let Some(owner) = countries.key(slot).map(|key| key as u32).filter(|&owner| wanted(owner)) else { continue };
            out.insert(owner, Producer { country: Totals::of(|at| countries.number(TOTALS[at], slot)), ..Producer::default() });
        }
        let types = hour.table(TableId::ProductionTypeTotals);
        for slot in types.rows() {
            let Some(key) = types.key(slot) else { continue };
            if let Some(producer) = out.get_mut(&((key >> 32) as u32)) {
                producer.types.insert(key as u32, Totals::of(|at| types.number(TYPES[at], slot)));
            }
        }
        if hour.complete(TableId::RocketProductionOutput) {
            let rocket = hour.table(TableId::RocketProductionOutput);
            let definitions = hour.table(TableId::VariantDefinitions);
            for slot in rocket.rows() {
                let Some(key) = rocket.key(slot) else { continue };
                let (owner, variant) = ((key >> 64) as u32, key as u64);
                if owner == 0 || variant == 0 {
                    continue;
                }
                let Some(producer) = out.get_mut(&owner) else { continue };
                let units = rocket.number(ROCKET[0], slot);
                let row = RocketRow { units: units.unwrap_or(0), units_exact: units.is_some() && rocket.number(ROCKET[1], slot) == Some(1), spent: rocket.number(ROCKET[2], slot).unwrap_or(0) };
                let token = definitions.slot(u128::from(variant)).and_then(|at| definitions.number(ARCHETYPE[0], at)).map_or(TypeTotal::NO_ARCHETYPE, |token| token as u32);
                producer.rocket.push((token, row));
            }
        }
        let coverage = hour.table(TableId::ProductionCoverage);
        for (&owner, producer) in &mut out {
            producer.own_rocket = covered_row(&coverage, coverage_key(owner, Coverage::ROCKET));
        }
        out
    }
}

impl Writer for ProducedValues {
    fn name(&self) -> &'static str {
        "ProducedValues"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        TABLES
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let run = Self::run(hour);
        Self::clock(hour, out, &run);
        let whole = hour.fresh || hour.first || self.rebuild || self.rocket_world != Some(run.rocket_world);
        let owners = if whole { None } else { Self::moved(hour) };
        if owners.as_ref().is_some_and(BTreeSet::is_empty) {
            return Ok(());
        }
        self.rebuild = false;
        self.rocket_world = Some(run.rocket_world);
        let producers = Self::producers(hour, owners.as_ref());
        let lost = owners.is_none() || [TableId::ProductionCountryTotals, TableId::ProductionTypeTotals, TableId::RocketProductionOutput].iter().any(|&table| hour.deleted_keys(table).next().is_some());
        for key in if lost { held(out, TableId::ProducedValues) } else { Vec::new() } {
            let (owner, archetype) = ((key >> 32) as u32, key as u32);
            if owner == Row::CLOCK || owners.as_ref().is_some_and(|owners| !owners.contains(&owner)) {
                continue;
            }
            let kept = producers.get(&owner).is_some_and(|producer| archetype == Row::WHOLE || archetype == Row::REST || parts(producer).contains(&archetype));
            if !kept {
                out.delete(TableId::ProducedValues, key);
            }
        }
        for (&owner, producer) in &producers {
            for part in [Part::Whole, Part::Rest].into_iter().chain(parts(producer).into_iter().map(Part::Archetype)) {
                out.set(TableId::ProducedValues, Row::key_of(owner, part.archetype()), row(producer, &run, part, hour.hour));
            }
        }
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, _blob: &[u8]) -> io::Result<()> {
        *self = Self { rebuild: true, ..Self::default() };
        Ok(())
    }
}
