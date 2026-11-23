
use recorder_derive_framework::framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns};
use recorder_derive_framework::deployed::{COUNTERS, DIVISION, Identity, LINK, SLOT, VOLUNTEER, invalid, number, row};
use replay_columnar::leaf::Writer as Bytes;
use replay_data::{
    military_aid,
    rows::{
        AirWingRow, CountryCounterInputRow, DivisionLandInputsRow, DivisionRow, DivisionTemplateIconRow, FleetRow, Recorded, ShipRow, TableId, TagHolderRow, TaskForceRow,
    },
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
};

#[cfg(test)]
mod tests;

const NONE: u32 = TagHolderRow::NONE;

const LAND: [usize; 1] = columns::<DivisionLandInputsRow, 1>(["manpower"]);
const TEMPLATE: [usize; 1] = columns::<DivisionTemplateIconRow, 1>(["manpower_exile"]);
const SHIP: [usize; 3] = columns::<ShipRow, 3>(["task_force", "acquired_manpower", "required_manpower"]);
const TASK: [usize; 1] = columns::<TaskForceRow, 1>(["fleet"]);
const FLEET: [usize; 1] = columns::<FleetRow, 1>(["owner"]);
const WING: [usize; 3] = columns::<AirWingRow, 3>(["owner", "acquired_manpower", "exile_country"]);

const fn read(table: TableId, columns: &'static [usize], required: bool) -> Read {
    Read { table, columns: Columns::These(columns), watch: Watch::Changed, required }
}

const ARMY: &[TableId] = &[TableId::Divisions, TableId::DivisionLandInputs, TableId::DivisionTemplateIcons, TableId::CountryCounterInputs, TableId::SentVolunteers];
const NAVY: &[TableId] = &[TableId::Ships, TableId::TaskForces, TableId::Fleets];
const AIR: &[TableId] = &[TableId::AirWings];

const READS: &[Read] = &[
    read(TableId::Countries, &SLOT, true),
    read(TableId::CountryLinks, &LINK, true),
    read(TableId::Divisions, &DIVISION, false),
    read(TableId::DivisionLandInputs, &LAND, false),
    read(TableId::DivisionTemplateIcons, &TEMPLATE, false),
    read(TableId::CountryCounterInputs, &COUNTERS, false),
    read(TableId::SentVolunteers, &VOLUNTEER, false),
    read(TableId::Ships, &SHIP, false),
    read(TableId::TaskForces, &TASK, false),
    read(TableId::Fleets, &FLEET, false),
    read(TableId::AirWings, &WING, false),
];

type Term = Option<i32>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Sum {
    total: i32,
    unknown: u32,
}

impl Sum {
    fn add(&mut self, term: Term) {
        match term {
            Some(value) => self.total = self.total.wrapping_add(value),
            None => self.unknown += 1,
        }
    }

    fn remove(&mut self, term: Term) {
        match term {
            Some(value) => self.total = self.total.wrapping_sub(value),
            None => self.unknown -= 1,
        }
    }

    fn value(self) -> Term {
        (self.unknown == 0).then_some(self.total)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Division {
    slot: u32,
    template: u64,
    term: Option<Term>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShipTerm {
    Owned { slot: u32, acquired: Term, required: Term },
    Unowned,
    Unresolved,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ship {
    task: u64,
    term: ShipTerm,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Wing {
    ordinary: Option<(u32, Term)>,
    exiled: Option<(u32, Term)>,
    unread: bool,
}

fn complete(hour: &Hour<'_>) -> [bool; 3] {
    [ARMY, NAVY, AIR].map(|tables| tables.iter().all(|&table| hour.complete(table)))
}

fn touched(hour: &Hour<'_>, table: TableId, columns: &'static [usize]) -> Vec<u128> {
    let view = hour.table(table);
    let mut keys: Vec<u128> = hour.deleted_keys(table).collect();
    keys.extend(hour.written_rows(table, Columns::These(columns)).into_iter().filter_map(|slot| view.key(slot)));
    keys
}

#[derive(Default)]
pub struct Deployed {
    rebuild: bool,
    complete: [bool; 3],
    identity: Identity,
    held: HashSet<u32>,
    divisions: HashMap<u64, Division>,
    by_template: HashMap<u64, Vec<u64>>,
    armies: HashMap<u32, Sum>,
    volunteers: HashMap<u32, BTreeMap<u32, u64>>,
    listed: HashMap<u32, (Term, Vec<u64>)>,
    users: HashMap<u64, Vec<u32>>,
    ships: HashMap<u64, Ship>,
    ships_of_task: HashMap<u64, Vec<u64>>,
    task_fleet: HashMap<u64, u64>,
    tasks_of_fleet: HashMap<u64, Vec<u64>>,
    navies: HashMap<u32, [Sum; 2]>,
    unresolved_ships: u32,
    wings: HashMap<u64, Wing>,
    crews: HashMap<u32, [Sum; 2]>,
    unread_exiles: u32,
}

fn put(index: &mut HashMap<u64, Vec<u64>>, at: u64, id: u64) {
    index.entry(at).or_default().push(id);
}

fn take(index: &mut HashMap<u64, Vec<u64>>, at: u64, id: u64) {
    if let Some(ids) = index.get_mut(&at) {
        if let Some(position) = ids.iter().position(|&held| held == id) {
            ids.swap_remove(position);
        }
        if ids.is_empty() {
            index.remove(&at);
        }
    }
}

impl Deployed {
    fn ordinary(hour: &Hour<'_>, template: u64) -> Option<bool> {
        let view = hour.table(TableId::DivisionTemplateIcons);
        let slot = view.slot(u128::from(template))?;
        view.bool(TEMPLATE[0], slot).map(|exile| !exile)
    }

    fn amount(&self, hour: &Hour<'_>, id: u64, slot: u32) -> io::Result<Term> {
        let view = hour.table(TableId::DivisionLandInputs);
        let Some(at) = view.slot(u128::from(id)) else { return Ok(None) };
        let row: DivisionLandInputsRow = row(&view, at, &LAND)?;
        Ok(Some(row.manpower.iter().find(|entry| self.identity.resolve(entry.country) == Some(slot)).map_or(0, |entry| entry.amount)))
    }

    fn division_term(&self, hour: &Hour<'_>, division: &DivisionRow) -> io::Result<(u32, Option<Term>)> {
        let Some(slot) = self.identity.resolve(division.roster_owner) else { return Ok((NONE, None)) };
        if self.identity.resolve(military_aid::owner(division)) != Some(slot) || self.identity.resolve(division.logical_owner) != Some(slot) {
            return Ok((slot, None));
        }
        Ok((
            slot,
            match Self::ordinary(hour, division.template) {
                None => Some(None),
                Some(false) => None,
                Some(true) => Some(self.amount(hour, division.id, slot)?),
            },
        ))
    }

    fn set_division(&mut self, hour: &Hour<'_>, id: u64, dirty: &mut HashSet<u32>) -> io::Result<()> {
        if let Some(old) = self.divisions.remove(&id) {
            take(&mut self.by_template, old.template, id);
            if let Some(term) = old.term {
                self.armies.entry(old.slot).or_default().remove(term);
            }
            dirty.insert(old.slot);
        }
        let view = hour.table(TableId::Divisions);
        let Some(at) = view.slot(u128::from(id)) else { return Ok(()) };
        let division: DivisionRow = row(&view, at, &DIVISION)?;
        let (slot, term) = self.division_term(hour, &division)?;
        if let Some(term) = term {
            self.armies.entry(slot).or_default().add(term);
        }
        put(&mut self.by_template, division.template, id);
        dirty.insert(slot);
        self.divisions.insert(id, Division { slot, template: division.template, term });
        Ok(())
    }

    fn lists(&self, hour: &Hour<'_>, slot: u32) -> io::Result<(Term, Vec<u64>)> {
        let aliases = self.identity.aliases(slot);
        let counters = hour.table(TableId::CountryCounterInputs);
        let expeditionaries = match aliases.iter().find_map(|&raw| counters.slot(u128::from(raw))) {
            None => None,
            Some(at) => {
                let row: CountryCounterInputRow = row(&counters, at, &COUNTERS)?;
                match row.sent_expeditionaries {
                    Recorded::Value(Some(list)) => Some(list.to_vec()),
                    _ => None,
                }
            }
        };
        let volunteers: Vec<u64> = aliases.iter().filter_map(|raw| self.volunteers.get(raw)).flat_map(|list| list.values().copied()).collect();
        let mut references: Vec<u64> = expeditionaries.iter().flatten().chain(&volunteers).copied().filter(|&id| id != 0).collect();
        references.sort_unstable();
        references.dedup();
        let Some(expeditionaries) = expeditionaries else { return Ok((None, references)) };
        let divisions = hour.table(TableId::Divisions);
        let template = |id: u64| -> Option<u64> { divisions.slot(u128::from(id)).and_then(|at| number(&divisions, DIVISION[3], at)) };
        let mut failed = None;
        let mut contribution = |id: u64| match self.amount(hour, id, slot) {
            Ok(term) => term,
            Err(error) => {
                failed = Some(error);
                None
            }
        };
        let mut sum = |list: &[u64]| military_aid::listed_manpower_with(list.iter().copied(), template, |template| Self::ordinary(hour, template), &mut contribution);
        let term = sum(&expeditionaries).zip(sum(&volunteers)).map(|(sent, volunteered)| sent.wrapping_add(volunteered));
        match failed {
            Some(error) => Err(error),
            None => Ok((term, references)),
        }
    }

    fn set_lists(&mut self, hour: &Hour<'_>, slot: u32) -> io::Result<()> {
        if let Some((_, references)) = self.listed.remove(&slot) {
            for id in references {
                if let Some(users) = self.users.get_mut(&id) {
                    users.retain(|&user| user != slot);
                    if users.is_empty() {
                        self.users.remove(&id);
                    }
                }
            }
        }
        if !self.identity.countries.contains(&slot) {
            return Ok(());
        }
        let (term, references) = self.lists(hour, slot)?;
        for &id in &references {
            self.users.entry(id).or_default().push(slot);
        }
        self.listed.insert(slot, (term, references));
        Ok(())
    }

    fn ship_term(hour: &Hour<'_>, identity: &Identity, task: u64, acquired: Term, required: Term) -> ShipTerm {
        let tasks = hour.table(TableId::TaskForces);
        let fleets = hour.table(TableId::Fleets);
        let Some(fleet) = tasks.slot(u128::from(task)).and_then(|at| number(&tasks, TASK[0], at)) else { return ShipTerm::Unresolved };
        let Some(owner) = fleets.slot(u128::from(fleet)).and_then(|at| number(&fleets, FLEET[0], at)) else { return ShipTerm::Unresolved };
        match identity.resolve(owner as u32) {
            Some(slot) => ShipTerm::Owned { slot, acquired, required },
            None => ShipTerm::Unowned,
        }
    }

    fn set_ship(&mut self, hour: &Hour<'_>, id: u64, dirty: &mut HashSet<u32>) {
        if let Some(old) = self.ships.remove(&id) {
            take(&mut self.ships_of_task, old.task, id);
            match old.term {
                ShipTerm::Owned { slot, acquired, required } => {
                    let sums = self.navies.entry(slot).or_default();
                    sums[0].remove(acquired);
                    sums[1].remove(required);
                    dirty.insert(slot);
                }
                ShipTerm::Unresolved => self.unresolved_ships -= 1,
                ShipTerm::Unowned => {}
            }
        }
        let view = hour.table(TableId::Ships);
        let Some(at) = view.slot(u128::from(id)) else { return };
        let crew = |column: usize| number(&view, column, at).map(|bits| bits as i32);
        let task = number(&view, SHIP[0], at).unwrap_or(0);
        let term = Self::ship_term(hour, &self.identity, task, crew(SHIP[1]), crew(SHIP[2]));
        match term {
            ShipTerm::Owned { slot, acquired, required } => {
                let sums = self.navies.entry(slot).or_default();
                sums[0].add(acquired);
                sums[1].add(required);
                dirty.insert(slot);
            }
            ShipTerm::Unresolved => self.unresolved_ships += 1,
            ShipTerm::Unowned => {}
        }
        put(&mut self.ships_of_task, task, id);
        self.ships.insert(id, Ship { task, term });
    }

    fn set_task(&mut self, hour: &Hour<'_>, task: u64) {
        if let Some(old) = self.task_fleet.remove(&task) {
            take(&mut self.tasks_of_fleet, old, task);
        }
        let view = hour.table(TableId::TaskForces);
        if let Some(fleet) = view.slot(u128::from(task)).and_then(|at| number(&view, TASK[0], at)) {
            self.task_fleet.insert(task, fleet);
            put(&mut self.tasks_of_fleet, fleet, task);
        }
    }

    fn set_wing(&mut self, hour: &Hour<'_>, id: u64, dirty: &mut HashSet<u32>) {
        if let Some(old) = self.wings.remove(&id) {
            for (at, part) in [old.ordinary, old.exiled].into_iter().enumerate() {
                if let Some((slot, term)) = part {
                    self.crews.entry(slot).or_default()[at].remove(term);
                    dirty.insert(slot);
                }
            }
            self.unread_exiles -= u32::from(old.unread);
        }
        let view = hour.table(TableId::AirWings);
        let Some(at) = view.slot(u128::from(id)) else { return };
        let owner = number(&view, WING[0], at).map(|bits| bits as u32);
        let acquired = number(&view, WING[1], at).map(|bits| bits as i32);
        let exile = number(&view, WING[2], at).map(|bits| bits as i32);
        let wing = Wing {
            ordinary: owner.and_then(|owner| self.identity.resolve(owner)).and_then(|slot| match exile {
                None => Some((slot, None)),
                Some(exile) if exile <= 0 => Some((slot, acquired)),
                Some(_) => None,
            }),
            exiled: exile.filter(|&exile| exile > 0).and_then(|exile| self.identity.resolve(exile as u32)).map(|slot| (slot, acquired)),
            unread: exile.is_none(),
        };
        for (at, part) in [wing.ordinary, wing.exiled].into_iter().enumerate() {
            if let Some((slot, term)) = part {
                self.crews.entry(slot).or_default()[at].add(term);
                dirty.insert(slot);
            }
        }
        self.unread_exiles += u32::from(wing.unread);
        self.wings.insert(id, wing);
    }

    fn rebuild(&mut self, hour: &Hour<'_>, out: &Rows<'_>) -> io::Result<()> {
        let held = std::mem::take(&mut self.held);
        *self = Self { identity: Identity::read(hour), complete: complete(hour), ..Self::default() };
        self.held = held;
        let mut ignored = HashSet::new();
        if self.complete[0] {
            let volunteers = hour.table(TableId::SentVolunteers);
            for slot in volunteers.rows() {
                let (Some(key), Some(division)) = (volunteers.key(slot), number(&volunteers, VOLUNTEER[0], slot)) else { continue };
                self.volunteers.entry((key >> 32) as u32).or_default().insert(key as u32, division);
            }
            let divisions = hour.table(TableId::Divisions);
            for slot in divisions.rows() {
                if let Some(key) = divisions.key(slot) {
                    self.set_division(hour, key as u64, &mut ignored)?;
                }
            }
            let mut countries: Vec<u32> = self.identity.countries.iter().copied().collect();
            countries.sort_unstable();
            for slot in countries {
                self.set_lists(hour, slot)?;
            }
        }
        if self.complete[1] {
            let tasks = hour.table(TableId::TaskForces);
            for slot in tasks.rows() {
                if let Some(key) = tasks.key(slot) {
                    self.set_task(hour, key as u64);
                }
            }
            let ships = hour.table(TableId::Ships);
            for slot in ships.rows() {
                if let Some(key) = ships.key(slot) {
                    self.set_ship(hour, key as u64, &mut ignored);
                }
            }
        }
        if self.complete[2] {
            let wings = hour.table(TableId::AirWings);
            for slot in wings.rows() {
                if let Some(key) = wings.key(slot) {
                    self.set_wing(hour, key as u64, &mut ignored);
                }
            }
        }
        let table = out.table(TableId::CountryDeployedManpower);
        self.held = table.rows().filter_map(|slot| table.key(slot)).map(|key| key as u32).collect();
        Ok(())
    }

    fn army(&mut self, hour: &Hour<'_>, dirty: &mut HashSet<u32>) -> io::Result<()> {
        let mut divisions: Vec<u64> = touched(hour, TableId::Divisions, &DIVISION).into_iter().map(|key| key as u64).collect();
        divisions.extend(touched(hour, TableId::DivisionLandInputs, &LAND).into_iter().map(|key| key as u64));
        for template in touched(hour, TableId::DivisionTemplateIcons, &TEMPLATE) {
            divisions.extend(self.by_template.get(&(template as u64)).into_iter().flatten().copied());
        }
        divisions.sort_unstable();
        divisions.dedup();
        let mut senders = HashSet::new();
        for &id in &divisions {
            self.set_division(hour, id, dirty)?;
            senders.extend(self.users.get(&id).into_iter().flatten().copied());
        }
        for key in touched(hour, TableId::CountryCounterInputs, &COUNTERS) {
            senders.extend(self.identity.resolve(key as u32));
        }
        for key in hour.deleted_keys(TableId::SentVolunteers) {
            let raw = (key >> 32) as u32;
            if let Some(list) = self.volunteers.get_mut(&raw) {
                list.remove(&(key as u32));
                if list.is_empty() {
                    self.volunteers.remove(&raw);
                }
            }
            senders.extend(self.identity.resolve(raw));
        }
        let volunteers = hour.table(TableId::SentVolunteers);
        for slot in hour.written_rows(TableId::SentVolunteers, Columns::These(&VOLUNTEER)) {
            let (Some(key), Some(division)) = (volunteers.key(slot), number(&volunteers, VOLUNTEER[0], slot)) else { continue };
            self.volunteers.entry((key >> 32) as u32).or_default().insert(key as u32, division);
            senders.extend(self.identity.resolve((key >> 32) as u32));
        }
        let mut senders: Vec<u32> = senders.into_iter().collect();
        senders.sort_unstable();
        for slot in senders {
            self.set_lists(hour, slot)?;
            dirty.insert(slot);
        }
        Ok(())
    }

    fn navy(&mut self, hour: &Hour<'_>, dirty: &mut HashSet<u32>) {
        let mut ships: Vec<u64> = touched(hour, TableId::Ships, &SHIP).into_iter().map(|key| key as u64).collect();
        for key in touched(hour, TableId::TaskForces, &TASK) {
            let task = key as u64;
            self.set_task(hour, task);
            ships.extend(self.ships_of_task.get(&task).into_iter().flatten().copied());
        }
        for key in touched(hour, TableId::Fleets, &FLEET) {
            for task in self.tasks_of_fleet.get(&(key as u64)).into_iter().flatten() {
                ships.extend(self.ships_of_task.get(task).into_iter().flatten().copied());
            }
        }
        ships.sort_unstable();
        ships.dedup();
        for id in ships {
            self.set_ship(hour, id, dirty);
        }
    }

    fn air(&mut self, hour: &Hour<'_>, dirty: &mut HashSet<u32>) {
        let mut wings: Vec<u64> = touched(hour, TableId::AirWings, &WING).into_iter().map(|key| key as u64).collect();
        wings.sort_unstable();
        wings.dedup();
        for id in wings {
            self.set_wing(hour, id, dirty);
        }
    }

    fn cells(&self, slot: u32) -> Vec<Value> {
        let cell = |term: Term| term.map_or(Value::Unrecorded, Value::I32);
        let army = self.complete[0]
            .then(|| {
                let roster = self.armies.get(&slot).copied().unwrap_or_default().value()?;
                let listed = self.listed.get(&slot).and_then(|(term, _)| *term)?;
                Some(roster.wrapping_add(listed))
            })
            .flatten();
        let navy = (self.complete[1] && self.unresolved_ships == 0).then(|| self.navies.get(&slot).copied().unwrap_or_default());
        let crews = self.complete[2].then(|| self.crews.get(&slot).copied().unwrap_or_default());
        vec![
            cell(army),
            cell(navy.and_then(|sums| sums[0].value())),
            cell(navy.and_then(|sums| sums[1].value())),
            cell(crews.and_then(|sums| sums[0].value())),
            cell(crews.filter(|_| self.unread_exiles == 0).and_then(|sums| sums[1].value())),
        ]
    }

    fn write(&mut self, dirty: HashSet<u32>, out: &mut Rows) {
        for slot in dirty {
            if self.identity.countries.contains(&slot) {
                out.set(TableId::CountryDeployedManpower, u128::from(slot), self.cells(slot));
                self.held.insert(slot);
            } else if self.held.remove(&slot) {
                out.delete(TableId::CountryDeployedManpower, u128::from(slot));
            }
        }
    }
}

impl Writer for Deployed {
    fn name(&self) -> &'static str {
        "Deployed"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::CountryDeployedManpower)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let mut dirty = HashSet::new();
        if hour.fresh || self.rebuild || Identity::changed(hour) || complete(hour) != self.complete {
            self.rebuild(hour, out)?;
            self.rebuild = false;
            dirty.extend(self.identity.countries.iter().copied());
            dirty.extend(self.held.iter().copied());
        } else {
            let globals = (self.unresolved_ships == 0, self.unread_exiles == 0);
            if self.complete[0] {
                self.army(hour, &mut dirty)?;
            }
            if self.complete[1] {
                self.navy(hour, &mut dirty);
            }
            if self.complete[2] {
                self.air(hour, &mut dirty);
            }
            if globals != (self.unresolved_ships == 0, self.unread_exiles == 0) {
                dirty.extend(self.identity.countries.iter().copied());
            }
        }
        dirty.remove(&NONE);
        self.write(dirty, out);
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("Deployed's state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}
