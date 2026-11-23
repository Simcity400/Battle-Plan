
use recorder_derive_framework::{
    forces::invalid,
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer},
};
use replay_columnar::{leaf::Writer as Bytes, store::TableView};
use replay_data::rows::{GroupSpanRow as Span, TableId};
use replay_derive::factories;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
};

const READS: &[Read] = &[
    Read { table: TableId::GroupSpans, columns: Columns::These(&[]), watch: Watch::Changed, required: true },
    Read { table: TableId::Groups, columns: Columns::These(&[0]), watch: Watch::Changed, required: true },
    Read { table: TableId::Countries, columns: Columns::These(&[0]), watch: Watch::Changed, required: true },
    Read { table: TableId::TerritoryTotals, columns: Columns::Every, watch: Watch::Changed, required: false },
    Read { table: TableId::CountryFactories, columns: Columns::These(&factories::READS), watch: Watch::Changed, required: false },
];

#[derive(Clone, Copy)]
enum Source {
    Territory(usize),
    Factories { mode: usize, mask: usize },
}

fn sources() -> io::Result<Vec<Source>> {
    let (table, territory) = (TableId::FactionTotals, TableId::TerritoryTotals);
    (0..table.field_count())
        .map(|field| {
            let name = table.field_name(field).ok_or_else(|| invalid(format!("FactionTotals has no field {field}")))?;
            if let Some(column) = (0..territory.field_count()).find(|&column| territory.field_name(column) == Some(name)) {
                return Ok(Source::Territory(column));
            }
            let (mode, mask) = name.split_at(name.len().saturating_sub(1));
            let mode = match mode {
                "free" => 0,
                "built" => 1,
                "top_bar" => 2,
                _ => return Err(invalid(format!("FactionTotals' {name} is neither territory nor a factory count"))),
            };
            match mask.parse::<usize>() {
                Ok(mask @ 1..=7) => Ok(Source::Factories { mode, mask }),
                _ => Err(invalid(format!("FactionTotals' {name} names no group mask"))),
            }
        })
        .collect()
}

#[derive(Default)]
pub struct FactionTotals {
    rebuild: bool,
    members: BTreeMap<u64, BTreeSet<u32>>,
    incomplete: HashSet<u64>,
    unobserved: bool,
    country: HashMap<u32, u32>,
    complete: (bool, bool),
    sources: Vec<Source>,
}

impl FactionTotals {
    fn read_rosters(&mut self, hour: &Hour<'_>) -> io::Result<()> {
        let groups = hour.table(TableId::Groups);
        let mut numbers: HashMap<u32, u64> = HashMap::new();
        for row in groups.rows() {
            let number = groups.key(row).ok_or_else(|| invalid(format!("Groups slot {row} holds no row")))? as u32;
            if let Some(faction) = groups.number(0, row) {
                numbers.insert(number, faction as u64);
            }
        }
        self.members.clear();
        self.incomplete.clear();
        self.unobserved = false;
        let spans = hour.table(TableId::GroupSpans);
        for row in spans.rows() {
            let key = spans.key(row).ok_or_else(|| invalid(format!("GroupSpans slot {row} holds no row")))?;
            let (kind, group, value) = ((key >> 96) as u8, (key >> 64) as u32, (key >> 32) as u32);
            match kind {
                Span::UNOBSERVED if group == Span::FACTIONS => self.unobserved = true,
                Span::MEMBER if group == Span::HUMANS => {}
                Span::MEMBER | Span::INCOMPLETE => {
                    let Some(&faction) = numbers.get(&group) else { continue };
                    if kind == Span::MEMBER {
                        self.members.entry(faction).or_default().insert(value);
                    } else {
                        self.incomplete.insert(faction);
                    }
                }
                _ => {}
            }
        }
        let countries = hour.table(TableId::Countries);
        self.country.clear();
        for row in countries.rows() {
            let id = countries.key(row).ok_or_else(|| invalid(format!("Countries slot {row} holds no row")))? as u32;
            if let Some(slot) = countries.number(0, row) {
                self.country.insert(slot as u32, id);
            }
        }
        Ok(())
    }

    fn factions_of(&self, slot: u32) -> impl Iterator<Item = u64> + '_ {
        self.members.iter().filter(move |(_, members)| members.contains(&slot)).map(|(&faction, _)| faction)
    }

    fn cells(&self, hour: &Hour<'_>, faction: u64) -> io::Result<Vec<Value>> {
        let members = &self.members[&faction];
        let known = !self.unobserved && !self.incomplete.contains(&faction);
        let views = [hour.table(TableId::TerritoryTotals), hour.table(TableId::CountryFactories)];
        let complete = [self.complete.0, self.complete.1];
        let mut out = Vec::with_capacity(self.sources.len());
        for source in &self.sources {
            let which = usize::from(matches!(source, Source::Factories { .. }));
            let mut sum = (known && complete[which]).then_some(0i64);
            for &slot in members {
                let Some(total) = sum else { break };
                let key = if which == 0 { Some(u128::from(slot)) } else { self.country.get(&slot).map(|&id| u128::from(id)) };
                sum = match key.and_then(|key| views[which].slot(key)) {
                    Some(row) => value(*source, &views[which], row).and_then(|value| total.checked_add(value)),
                    None => None,
                };
            }
            out.push(sum.map_or(Value::Unrecorded, Value::I64));
        }
        Ok(out)
    }
}

fn value(source: Source, view: &TableView<'_>, row: u32) -> Option<i64> {
    match source {
        Source::Territory(column) => view.number(column, row),
        Source::Factories { mode, mask } => factories::channel(&|column| view.number(column, row), mode, mask),
    }
}

impl Writer for FactionTotals {
    fn name(&self) -> &'static str {
        "FactionTotals"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::FactionTotals)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let table = TableId::FactionTotals;
        if self.sources.is_empty() {
            self.sources = sources()?;
        }
        let complete = (hour.complete(TableId::TerritoryTotals), hour.complete(TableId::CountryFactories));
        let rosters = hour.wrote(TableId::GroupSpans, Columns::These(&[])) || hour.wrote(TableId::Groups, Columns::These(&[0]))
            || hour.wrote(TableId::Countries, Columns::These(&[0]));
        let mut dirty: BTreeSet<u64> = BTreeSet::new();
        if hour.fresh || self.rebuild || rosters || complete != self.complete {
            self.rebuild = false;
            self.complete = complete;
            let held: Vec<u64> = recorder_derive_framework::forces::held(out, table).into_iter().map(|key| key as u64).collect();
            self.read_rosters(hour)?;
            dirty.extend(held);
            dirty.extend(self.members.keys().copied());
        } else {
            let territory = hour.table(TableId::TerritoryTotals);
            let factories = hour.table(TableId::CountryFactories);
            let mut slots: BTreeSet<u32> = BTreeSet::new();
            for row in hour.written_rows(TableId::TerritoryTotals, Columns::Every) {
                slots.extend(territory.key(row).map(|key| key as u32));
            }
            slots.extend(hour.deleted_keys(TableId::TerritoryTotals).map(|key| key as u32));
            let by_id: HashMap<u32, u32> = self.country.iter().map(|(&slot, &id)| (id, slot)).collect();
            let ids = hour
                .written_rows(TableId::CountryFactories, Columns::These(&factories::READS))
                .into_iter()
                .filter_map(|row| factories.key(row))
                .chain(hour.deleted_keys(TableId::CountryFactories));
            slots.extend(ids.filter_map(|key| by_id.get(&(key as u32)).copied()));
            for slot in slots {
                dirty.extend(self.factions_of(slot));
            }
        }
        for faction in dirty {
            let key = u128::from(faction);
            if self.members.get(&faction).is_some_and(|members| !members.is_empty()) {
                let cells = self.cells(hour, faction)?;
                out.set(table, key, cells);
            } else if out.table(table).slot(key).is_some() {
                out.delete(table, key);
            }
        }
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("FactionTotals' state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
