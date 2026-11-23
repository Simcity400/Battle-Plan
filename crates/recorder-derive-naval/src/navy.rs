
use recorder_derive_framework::{
    forces::{self, Owners, exact, invalid, number, written},
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns},
};
use replay_columnar::{leaf::Writer as Bytes, store::TableView};
use replay_data::{
    format::FormatVersion,
    rows::{RowCodec, ShipModelRow, ShipRow, TableId, TaskForceRow},
};
use std::{
    collections::{HashMap, HashSet},
    io,
};

const SHIP_COLUMNS: [usize; 3] = columns::<ShipRow, 3>(["task_force", "strength", "class_id"]);
const FORCE_COLUMNS: [usize; 2] = columns::<TaskForceRow, 2>(["roster_owner", "logical_owner"]);
const MODEL_COLUMNS: [usize; 1] = columns::<ShipModelRow, 1>(["counter"]);

const LEAST_MAXIMUM: i64 = 100;

const SIDES: usize = 2;

const READS: &[Read] = &[
    Read { table: TableId::Ships, columns: Columns::These(&SHIP_COLUMNS), watch: Watch::Changed, required: true },
    Read { table: TableId::TaskForces, columns: Columns::These(&FORCE_COLUMNS), watch: Watch::Changed, required: true },
    Read { table: TableId::ShipModels, columns: Columns::These(&MODEL_COLUMNS), watch: Watch::Changed, required: false },
    Read { table: TableId::Countries, columns: Columns::These(&[0]), watch: Watch::Changed, required: true },
    Read { table: TableId::CountryLinks, columns: Columns::These(&[0]), watch: Watch::Changed, required: true },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ship {
    force: u64,
    owners: [Option<u32>; SIDES],
    hp: Option<i64>,
    maximum: Option<i64>,
    class: u32,
}

#[derive(Clone, Copy, Default)]
struct Class {
    ships: i64,
    hp: i128,
    negative: i64,
}

#[derive(Default)]
struct Side {
    ships: i64,
    hp: i128,
    negative: i64,
    maximum: i128,
    missing: i64,
    classes: HashMap<u32, Class>,
}

#[derive(Default)]
pub struct Navy {
    rebuild: bool,
    owners: Owners,
    forces: HashMap<u64, [u32; SIDES]>,
    fleets: HashMap<u64, Vec<u64>>,
    maxima: Option<HashMap<u64, Option<i64>>>,
    ships: HashMap<u64, Ship>,
    sides: HashMap<(u32, usize), Side>,
    unresolved: [i64; SIDES],
    dirty: HashSet<u32>,
    dirty_classes: HashSet<(u32, usize, u32)>,
}

fn known(unresolved: [i64; SIDES]) -> [bool; SIDES] {
    unresolved.map(|count| count == 0)
}

fn class_key(slot: u32, side: usize, class: u32) -> u128 {
    (u128::from(slot) << 40) | ((side as u128) << 32) | u128::from(class)
}

fn class_of(key: u128) -> (u32, usize, u32) {
    ((key >> 40) as u32, ((key >> 32) & 0xff) as usize, key as u32)
}

fn model(view: &TableView<'_>, slot: u32) -> io::Result<(u64, Option<i64>)> {
    let key = view.key(slot).ok_or_else(|| invalid(format!("ShipModels slot {slot} holds no row")))?;
    let format = FormatVersion::CURRENT.version();
    let row = ShipModelRow::from_wire_fields(key, 1 << MODEL_COLUMNS[0], &mut |column| replay_frame::frame::field(view, slot, column, format))?;
    Ok((row.ship, row.counter.map(|counter| counter.maximum_strength.max(LEAST_MAXIMUM))))
}

impl Navy {
    fn count(&mut self, ship: &Ship, sign: i64) {
        for (at, owner) in ship.owners.iter().enumerate() {
            let Some(slot) = *owner else {
                self.unresolved[at] += sign;
                continue;
            };
            self.dirty.insert(slot);
            self.dirty_classes.insert((slot, at, ship.class));
            let side = self.sides.entry((slot, at)).or_default();
            side.ships += sign;
            let negative = sign * i64::from(ship.hp.is_none());
            let hp = i128::from(sign) * i128::from(ship.hp.unwrap_or(0));
            side.hp += hp;
            side.negative += negative;
            match ship.maximum {
                Some(maximum) => side.maximum += i128::from(sign) * i128::from(maximum),
                None => side.missing += sign,
            }
            let class = side.classes.entry(ship.class).or_default();
            class.ships += sign;
            class.hp += hp;
            class.negative += negative;
            if class.ships == 0 {
                side.classes.remove(&ship.class);
            }
            if side.ships == 0 {
                self.sides.remove(&(slot, at));
            }
        }
    }

    fn set(&mut self, id: u64, ship: Option<Ship>) {
        let old = self.ships.get(&id).copied();
        if old == ship {
            return;
        }
        if let Some(old) = &old {
            self.count(old, -1);
        }
        if let Some(new) = &ship {
            self.count(new, 1);
        }
        let (from, to) = (old.map(|ship| ship.force), ship.map(|ship| ship.force));
        if from != to {
            if let Some(force) = from
                && let Some(ids) = self.fleets.get_mut(&force)
            {
                ids.retain(|&held| held != id);
                if ids.is_empty() {
                    self.fleets.remove(&force);
                }
            }
            if let Some(force) = to {
                self.fleets.entry(force).or_default().push(id);
            }
        }
        match ship {
            Some(ship) => self.ships.insert(id, ship),
            None => self.ships.remove(&id),
        };
    }

    fn ship(&self, view: &TableView<'_>, slot: u32) -> io::Result<(u64, Ship)> {
        let table = TableId::Ships;
        let id = view.key(slot).ok_or_else(|| invalid(format!("Ships slot {slot} holds no row")))? as u64;
        let force = number(view, table, SHIP_COLUMNS[0], slot)? as u64;
        let strength = number(view, table, SHIP_COLUMNS[1], slot)?;
        let class = number(view, table, SHIP_COLUMNS[2], slot)? as i32 as u32;
        let owners = self.forces.get(&force).map_or([None; SIDES], |raw| raw.map(|raw| self.owners.slot(raw)));
        let maximum = self.maxima.as_ref().and_then(|maxima| maxima.get(&id).copied().flatten());
        Ok((id, Ship { force, owners, hp: (strength >= 0).then_some(strength), maximum, class }))
    }

    fn force(view: &TableView<'_>, slot: u32) -> io::Result<(u64, [u32; SIDES])> {
        let table = TableId::TaskForces;
        let id = view.key(slot).ok_or_else(|| invalid(format!("TaskForces slot {slot} holds no row")))? as u64;
        Ok((id, [number(view, table, FORCE_COLUMNS[0], slot)? as u32, number(view, table, FORCE_COLUMNS[1], slot)? as u32]))
    }

    fn rebuild(&mut self, hour: &Hour<'_>) -> io::Result<()> {
        self.owners.read_links(hour);
        self.owners.read_countries(hour);
        self.forces.clear();
        self.fleets.clear();
        self.ships.clear();
        self.sides.clear();
        self.unresolved = [0; SIDES];
        let forces = hour.table(TableId::TaskForces);
        for slot in forces.rows() {
            let (id, raw) = Self::force(&forces, slot)?;
            self.forces.insert(id, raw);
        }
        self.maxima = match hour.complete(TableId::ShipModels) {
            true => {
                let models = hour.table(TableId::ShipModels);
                let mut maxima = HashMap::with_capacity(models.live().count());
                for slot in models.rows() {
                    let (ship, maximum) = model(&models, slot)?;
                    maxima.insert(ship, maximum);
                }
                Some(maxima)
            }
            false => None,
        };
        let ships = hour.table(TableId::Ships);
        for slot in ships.rows() {
            let (id, ship) = self.ship(&ships, slot)?;
            self.set(id, Some(ship));
        }
        Ok(())
    }

    fn apply(&mut self, hour: &Hour<'_>) -> io::Result<bool> {
        if forces::links_moved(hour) || forces::countries_moved(hour) || hour.complete(TableId::ShipModels) != self.maxima.is_some() {
            return Ok(false);
        }
        let mut changed: Vec<u64> = Vec::new();
        let mut moved: Vec<u64> = Vec::new();
        for key in hour.deleted_keys(TableId::TaskForces) {
            if self.forces.remove(&(key as u64)).is_some() {
                moved.push(key as u64);
            }
        }
        let forces = hour.table(TableId::TaskForces);
        for slot in written(hour, TableId::TaskForces, &FORCE_COLUMNS) {
            let (id, raw) = Self::force(&forces, slot)?;
            if self.forces.insert(id, raw) != Some(raw) {
                moved.push(id);
            }
        }
        for force in moved {
            changed.extend(self.fleets.get(&force).into_iter().flatten().copied());
        }
        if let Some(maxima) = &mut self.maxima {
            for key in hour.deleted_keys(TableId::ShipModels) {
                if maxima.remove(&(key as u64)).is_some() {
                    changed.push(key as u64);
                }
            }
            let models = hour.table(TableId::ShipModels);
            for slot in written(hour, TableId::ShipModels, &MODEL_COLUMNS) {
                let (ship, maximum) = model(&models, slot)?;
                if maxima.insert(ship, maximum) != Some(maximum) {
                    changed.push(ship);
                }
            }
        }
        for key in hour.deleted_keys(TableId::Ships) {
            self.set(key as u64, None);
        }
        let ships = hour.table(TableId::Ships);
        changed.extend(written(hour, TableId::Ships, &SHIP_COLUMNS).into_iter().filter_map(|slot| ships.key(slot)).map(|key| key as u64));
        changed.sort_unstable();
        changed.dedup();
        for id in changed {
            let Some(slot) = ships.slot(u128::from(id)) else { continue };
            let (id, ship) = self.ship(&ships, slot)?;
            self.set(id, Some(ship));
        }
        Ok(true)
    }

    fn row(&self, slot: u32) -> Option<Vec<Value>> {
        let held = [self.sides.get(&(slot, 0)), self.sides.get(&(slot, 1))];
        if !self.owners.countries().contains(&slot) && held.iter().all(Option::is_none) {
            return None;
        }
        let mut cells = vec![Value::Unrecorded; 3 * SIDES];
        for (at, known) in known(self.unresolved).into_iter().enumerate() {
            if !known {
                continue;
            }
            let side = held[at];
            cells[3 * at] = Value::I64(side.map_or(0, |side| side.ships));
            if side.is_none_or(|side| side.negative == 0) {
                cells[3 * at + 1] = exact(side.map_or(0, |side| side.hp));
            }
            if self.maxima.is_some() && side.is_none_or(|side| side.missing == 0) {
                cells[3 * at + 2] = exact(side.map_or(0, |side| side.maximum));
            }
        }
        Some(cells)
    }

    fn class(&self, key: u128) -> Option<Vec<Value>> {
        let (slot, at, class) = class_of(key);
        if self.unresolved.get(at).is_none_or(|&count| count != 0) {
            return None;
        }
        let held = self.sides.get(&(slot, at))?.classes.get(&class)?;
        Some(vec![Value::I64(held.ships), if held.negative == 0 { exact(held.hp) } else { Value::Unrecorded }])
    }
}

impl Writer for Navy {
    fn name(&self) -> &'static str {
        "Navy"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::NavyTotals), Output::State(TableId::NavyClasses)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let before = known(self.unresolved);
        let applied = !hour.fresh && !self.rebuild && self.apply(hour)?;
        if !applied {
            self.rebuild(hour)?;
            self.rebuild = false;
        }
        let all = !applied || before != known(self.unresolved);
        let dirty = std::mem::take(&mut self.dirty);
        let dirty_classes = std::mem::take(&mut self.dirty_classes);
        let (slots, classes): (Vec<u128>, Vec<u128>) = match all {
            true => (
                self.owners.countries().iter().copied().chain(self.sides.keys().map(|&(slot, _)| slot)).map(u128::from).collect(),
                self.sides.iter().flat_map(|(&(slot, at), side)| side.classes.keys().map(move |&class| class_key(slot, at, class))).collect(),
            ),
            false => (dirty.into_iter().map(u128::from).collect(), dirty_classes.into_iter().map(|(slot, at, class)| class_key(slot, at, class)).collect()),
        };
        forces::write(out, TableId::NavyTotals, slots, all, |key| self.row(key as u32));
        forces::write(out, TableId::NavyClasses, classes, all, |key| self.class(key));
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("Navy's state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
