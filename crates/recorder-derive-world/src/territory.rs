
use recorder_derive_framework::framework::{Columns, Definition, Hour, Output, Read, Rows, Uses, Value, Watch, Writer};
use replay_columnar::{
    leaf::Writer as Bytes,
    state::Kind,
    store::{TableView, Value as Held},
};
use replay_data::rows::{TableId, TerritoryTotalRow};
use std::{
    collections::{HashMap, HashSet},
    io,
};

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

const STATE_OWNER: usize = 0;
const STATE_CONTROLLER: usize = 1;
const PROVINCE_OWNER: usize = 1;
const PROVINCE_CONTROLLER: usize = 2;
const PROVINCE_POINTS: usize = 3;
const STATE_COLUMNS: &[usize] = &[STATE_OWNER, STATE_CONTROLLER];
const PROVINCE_COLUMNS: &[usize] = &[PROVINCE_OWNER, PROVINCE_CONTROLLER, PROVINCE_POINTS];

const READS: &[Read] = &[
    Read { table: TableId::States, columns: Columns::These(STATE_COLUMNS), watch: Watch::Changed, required: true },
    Read { table: TableId::Provinces, columns: Columns::These(PROVINCE_COLUMNS), watch: Watch::Changed, required: true },
    Read { table: TableId::CountryLinks, columns: Columns::These(&[0]), watch: Watch::Changed, required: true },
    Read { table: TableId::Countries, columns: Columns::These(&[COUNTRY_SLOT]), watch: Watch::Changed, required: true },
];

const COUNTRY_SLOT: usize = 0;
const WORLD: u32 = TerritoryTotalRow::WORLD;

const KINDS: usize = 2;
const RULES: usize = 3;

const USES: &[Uses] = &[Uses { definition: Definition::ProvinceAreas, required: false }];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Place {
    owner: u32,
    controller: u32,
    points: i64,
    area: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Totals {
    places: [[i64; RULES]; KINDS],
    points: [i64; RULES],
    area: [i64; RULES],
    unmeasured: [i64; RULES],
}

impl Totals {
    fn holds(&self) -> bool {
        self.places.iter().flatten().any(|&count| count != 0)
    }
}

#[derive(Default)]
pub struct Territory {
    rebuild: bool,
    links: HashMap<u32, u32>,
    places: [HashMap<u32, Place>; KINDS],
    totals: HashMap<u32, Totals>,
    world: Totals,
    countries: HashSet<u32>,
    unresolved: [[i64; 2]; KINDS],
    held: HashSet<u32>,
    unmapped: Option<String>,
}

fn cell(table: &TableView<'_>, column: usize, slot: u32) -> Option<i64> {
    match (table.kinds().get(column)?, table.value(column, slot)) {
        (Kind::U32 | Kind::I32, Held::Number(bits)) => Some(bits as i64),
        _ => None,
    }
}

const TABLES: [TableId; KINDS] = [TableId::States, TableId::Provinces];

fn place(table: &TableView<'_>, kind: usize, slot: u32, id: u32, areas: &[i64]) -> Place {
    let (owner, controller) = if kind == 0 { (STATE_OWNER, STATE_CONTROLLER) } else { (PROVINCE_OWNER, PROVINCE_CONTROLLER) };
    Place {
        owner: cell(table, owner, slot).unwrap_or(0) as u32,
        controller: cell(table, controller, slot).unwrap_or(0) as u32,
        points: if kind == 1 { cell(table, PROVINCE_POINTS, slot).unwrap_or(0) } else { 0 },
        area: if kind == 1 { areas.get(id as usize).copied() } else { None },
    }
}

impl Territory {
    fn slot(&self, raw: u32) -> Result<Option<u32>, ()> {
        match raw {
            0 => Ok(None),
            raw => self.links.get(&raw).copied().map(Some).ok_or(()),
        }
    }

    fn count(&mut self, kind: usize, place: Place, sign: i64, dirty: &mut HashSet<u32>) {
        let owner = self.slot(place.owner);
        let controller = self.slot(place.controller);
        if owner.is_err() {
            self.unresolved[kind][0] += sign;
        }
        if controller.is_err() {
            self.unresolved[kind][1] += sign;
        }
        let mut credit = |slot: u32, rule: usize| {
            let totals = self.totals.entry(slot).or_default();
            totals.places[kind][rule] += sign;
            if kind == 1 {
                totals.points[rule] += sign * place.points;
                match place.area {
                    Some(area) => totals.area[rule] += sign * area,
                    None => totals.unmeasured[rule] += sign,
                }
            }
            dirty.insert(slot);
        };
        if let Ok(Some(slot)) = owner {
            credit(slot, 0);
        }
        let occupied = matches!((owner, controller), (Ok(Some(held)), Ok(Some(by))) if held != by);
        if let Ok(Some(slot)) = controller {
            credit(slot, 1);
            if occupied {
                credit(slot, 2);
            }
        }
        for rule in 0..RULES {
            if rule == 2 && !occupied {
                continue;
            }
            self.world.places[kind][rule] += sign;
            if kind == 1 {
                self.world.points[rule] += sign * place.points;
                match place.area {
                    Some(area) => self.world.area[rule] += sign * area,
                    None => self.world.unmeasured[rule] += sign,
                }
            }
        }
        dirty.insert(WORLD);
    }

    fn rebuild(&mut self, hour: &Hour<'_>, out: &Rows<'_>) -> io::Result<()> {
        let areas = hour.definitions().areas();
        self.unmapped = areas.err().map(str::to_owned);
        let areas = areas.unwrap_or_default();
        self.links.clear();
        let links = hour.table(TableId::CountryLinks);
        for slot in links.rows() {
            let (Some(key), Some(canonical)) = (links.key(slot), cell(&links, 0, slot)) else { continue };
            self.links.insert(key as u32, canonical as u32);
        }
        self.countries.clear();
        let countries = hour.table(TableId::Countries);
        self.countries.extend(countries.rows().filter_map(|slot| cell(&countries, COUNTRY_SLOT, slot)).map(|slot| slot as u32));
        self.totals.clear();
        self.world = Totals::default();
        self.unresolved = [[0; 2]; KINDS];
        let mut ignored = HashSet::new();
        for kind in 0..KINDS {
            let held = hour.table(TABLES[kind]);
            let mut places = HashMap::with_capacity(held.live().count());
            for slot in held.rows() {
                let Some(key) = held.key(slot) else { continue };
                places.insert(key as u32, place(&held, kind, slot, key as u32, areas));
            }
            for &place in places.values() {
                self.count(kind, place, 1, &mut ignored);
            }
            self.places[kind] = places;
        }
        self.held.clear();
        let derived = out.table(TableId::TerritoryTotals);
        self.held.extend(derived.rows().filter_map(|slot| derived.key(slot)).map(|key| key as u32));
        Ok(())
    }

    fn apply(&mut self, hour: &Hour<'_>, dirty: &mut HashSet<u32>) -> io::Result<()> {
        let areas = hour.definitions().areas().unwrap_or_default();
        for kind in 0..KINDS {
            let held = hour.table(TABLES[kind]);
            let mut edited: HashMap<u32, Option<Place>> = HashMap::new();
            for key in hour.deleted_keys(TABLES[kind]) {
                edited.insert(key as u32, None);
            }
            let columns = if kind == 0 { STATE_COLUMNS } else { PROVINCE_COLUMNS };
            for slot in hour.written_rows(TABLES[kind], Columns::These(columns)) {
                if let Some(key) = held.key(slot) {
                    edited.insert(key as u32, Some(place(&held, kind, slot, key as u32, areas)));
                }
            }
            for (id, now) in edited {
                let before = self.places[kind].get(&id).copied();
                if before == now {
                    continue;
                }
                if let Some(before) = before {
                    self.count(kind, before, -1, dirty);
                }
                match now {
                    Some(now) => {
                        self.count(kind, now, 1, dirty);
                        self.places[kind].insert(id, now);
                    }
                    None => {
                        self.places[kind].remove(&id);
                    }
                }
            }
        }
        Ok(())
    }

    fn cells(&self, slot: u32) -> Vec<Value> {
        let world = slot == WORLD;
        let totals = if world { self.world } else { self.totals.get(&slot).copied().unwrap_or_default() };
        let known = |kind: usize, rule: usize| match rule {
            0 => world || self.unresolved[kind][0] == 0,
            1 => world || self.unresolved[kind][1] == 0,
            _ => self.unresolved[kind][0] == 0 && self.unresolved[kind][1] == 0,
        };
        let value = |known: bool, value: i64| if known { Value::I64(value) } else { Value::Unrecorded };
        let mut cells = Vec::with_capacity(RULES * 4);
        for kind in 0..KINDS {
            for rule in 0..RULES {
                cells.push(value(known(kind, rule), totals.places[kind][rule]));
            }
        }
        for rule in 0..RULES {
            cells.push(value(known(1, rule), totals.points[rule]));
        }
        for rule in 0..RULES {
            cells.push(value(known(1, rule) && self.unmapped.is_none() && totals.unmeasured[rule] == 0, totals.area[rule]));
        }
        cells
    }

    fn kept(&self, slot: u32) -> bool {
        match slot {
            WORLD => self.world.holds(),
            slot => self.countries.contains(&slot) || self.totals.get(&slot).is_some_and(Totals::holds),
        }
    }

    fn write(&mut self, dirty: HashSet<u32>, out: &mut Rows) {
        for slot in dirty {
            if self.kept(slot) {
                out.set(TableId::TerritoryTotals, u128::from(slot), self.cells(slot));
                self.held.insert(slot);
            } else {
                self.totals.remove(&slot);
                if self.held.remove(&slot) {
                    out.delete(TableId::TerritoryTotals, u128::from(slot));
                }
            }
        }
    }
}

impl Writer for Territory {
    fn name(&self) -> &'static str {
        "Territory"
    }

    fn version(&self) -> u32 {
        3
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::TerritoryTotals)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn definitions(&self) -> &'static [Uses] {
        USES
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let unresolved = self.unresolved;
        let mut dirty = HashSet::new();
        let countries = hour.wrote(TableId::Countries, Columns::These(&[COUNTRY_SLOT]));
        if hour.fresh || self.rebuild || countries || hour.wrote(TableId::CountryLinks, Columns::These(&[0])) {
            self.rebuild(hour, out)?;
            self.rebuild = false;
            dirty.extend(self.totals.keys().copied());
            dirty.extend(self.held.iter().copied());
            dirty.extend(self.countries.iter().copied());
            dirty.insert(WORLD);
        } else {
            self.apply(hour, &mut dirty)?;
            if self.unresolved != unresolved {
                dirty.extend(self.held.iter().copied());
            }
        }
        self.write(dirty, out);
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("Territory's state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn territory() -> Territory {
        Territory { links: [(10, 1), (20, 2), (30, 3)].into(), ..Territory::default() }
    }

    fn province(owner: u32, controller: u32, points: i64, area: Option<i64>) -> Place {
        Place { owner, controller, points, area }
    }

    fn state(owner: u32, controller: u32) -> Place {
        Place { owner, controller, ..Place::default() }
    }

    #[test]
    fn owners_controllers_and_occupiers_are_credited_and_zero_counts_for_no_one() {
        let mut territory = territory();
        let mut dirty = HashSet::new();
        territory.count(1, province(10, 20, 5, Some(7)), 1, &mut dirty);
        territory.count(0, state(10, 10), 1, &mut dirty);
        territory.count(0, state(0, 0), 1, &mut dirty);
        assert_eq!(dirty, [1, 2, WORLD].into());
        let owner = territory.cells(1);
        let occupier = territory.cells(2);
        let i = |value: i64| Value::I64(value);
        assert_eq!(owner, vec![i(1), i(1), i(0), i(1), i(0), i(0), i(5), i(0), i(0), i(7), i(0), i(0)]);
        assert_eq!(occupier, vec![i(0), i(0), i(0), i(0), i(1), i(1), i(0), i(5), i(5), i(0), i(7), i(7)]);
        territory.count(1, province(10, 20, 5, Some(7)), -1, &mut dirty);
        assert!(!territory.totals[&2].holds(), "a slot holding nothing has no row");
    }

    #[test]
    fn an_owner_no_link_resolves_makes_owned_and_occupied_unknown_never_zero() {
        let mut territory = territory();
        let mut dirty = HashSet::new();
        territory.count(0, state(99, 30), 1, &mut dirty);
        let cells = territory.cells(3);
        assert_eq!(cells[0], Value::Unrecorded, "owned states");
        assert_eq!(cells[1], Value::I64(1), "controlled states stay known");
        assert_eq!(cells[2], Value::Unrecorded, "occupied states");
        assert_eq!(cells[3], Value::I64(0), "provinces read no unresolved owner");
    }

    #[test]
    fn area_totals_are_the_hand_count_and_a_province_without_area_makes_them_unknown() {
        let mut territory = territory();
        let mut dirty = HashSet::new();
        let areas = [0, 40, 25, 10];
        for (id, owner, controller) in [(1, 10, 10), (2, 10, 10), (3, 10, 20)] {
            let place = Place { owner, controller, points: 0, area: areas.get(id).copied() };
            territory.count(1, place, 1, &mut dirty);
        }
        let i = |value: i64| Value::I64(value);
        assert_eq!(territory.cells(1)[9..], [i(75), i(65), i(0)], "owned 40 + 25 + 10; controlled 40 + 25");
        assert_eq!(territory.cells(2)[9..], [i(0), i(10), i(10)], "slot 2 controls and occupies province 3");
        let place = Place { owner: 30, controller: 30, points: 0, area: areas.get(4).copied() };
        territory.count(1, place, 1, &mut dirty);
        assert_eq!(territory.cells(3)[9..], [Value::Unrecorded, Value::Unrecorded, i(0)]);
        assert_eq!(territory.cells(3)[3], i(1), "the province still counts");
        assert_eq!(territory.cells(1)[9..], [i(75), i(65), i(0)]);
        territory.unmapped = Some("the session's game facts are not packed yet".into());
        assert!(territory.cells(1)[9..].iter().all(|cell| *cell == Value::Unrecorded));
        assert_eq!(territory.cells(1)[3], i(3), "the other columns stay known");
    }

    #[test]
    fn the_world_counts_every_place_and_a_live_country_keeps_a_row_of_zeros() {
        let mut territory = territory();
        territory.countries = [1, 2, 4].into();
        let mut dirty = HashSet::new();
        territory.count(0, state(10, 20), 1, &mut dirty);
        territory.count(0, state(0, 0), 1, &mut dirty);
        territory.count(1, province(10, 10, 3, Some(5)), 1, &mut dirty);
        let i = |value: i64| Value::I64(value);
        assert_eq!(territory.cells(WORLD), vec![i(2), i(2), i(1), i(1), i(1), i(0), i(3), i(3), i(0), i(5), i(5), i(0)]);
        assert!(territory.kept(WORLD) && territory.kept(4) && !territory.kept(3));
        assert!(territory.cells(4).iter().all(|cell| *cell == i(0)), "a live country holding nothing holds recorded zeros");
        territory.count(0, state(99, 10), 1, &mut dirty);
        let world = territory.cells(WORLD);
        assert_eq!((world[0], world[1]), (i(3), i(3)), "the world's owned and controlled need no link");
        assert_eq!(world[2], Value::Unrecorded, "who occupies is not known while an owner is not");
    }

    #[test]
    fn its_state_is_its_tables_so_a_resume_reads_them_again() {
        let mut territory = Territory::default();
        let mut blob = Bytes::new();
        territory.save(&mut blob).unwrap();
        assert!(blob.bytes().is_empty());
        territory.load(&[]).unwrap();
        assert!(territory.rebuild);
        assert!(territory.load(&[1]).is_err());
    }
}
