
use recorder_derive_framework::{
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer},
    producers::{Producers, SLOT},
};
use replay_columnar::leaf::Writer as Bytes;
use replay_data::rows::{DockyardRepairRateRow as Rate, TableId};
use replay_derive::{
    naval_repairs::{self, Operands, QueuedForce, QueuedModel, QueuedShip, Unavailable},
    visibility::{AccessLinks, Relations},
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
};

mod inputs;
#[cfg(test)]
mod tests;

use inputs::*;

const READS: &[Read] = &[
    Read { table: TableId::Countries, columns: Columns::These(&SLOT), watch: Watch::Changed, required: true },
    Read { table: TableId::CountryLinks, columns: Columns::These(&SLOT), watch: Watch::Changed, required: true },
    Read { table: TableId::NavalRepairCountries, columns: Columns::These(&COUNT), watch: Watch::Changed, required: false },
    Read { table: TableId::CountryProductionModifiers, columns: Columns::These(&MODIFIERS), watch: Watch::Changed, required: false },
    Read { table: TableId::NavalBases, columns: Columns::These(&BASE), watch: Watch::Changed, required: false },
    Read { table: TableId::NavalRepairBases, columns: Columns::These(&ALLOCATION), watch: Watch::Changed, required: false },
    Read { table: TableId::NavalRepairQueue, columns: Columns::These(&QUEUE), watch: Watch::Changed, required: false },
    Read { table: TableId::Ships, columns: Columns::These(&SHIP), watch: Watch::Changed, required: false },
    Read { table: TableId::ShipModels, columns: Columns::These(&MODEL), watch: Watch::Changed, required: false },
    Read { table: TableId::TaskForces, columns: Columns::These(&FORCE), watch: Watch::Changed, required: false },
    Read { table: TableId::Provinces, columns: Columns::These(&CONTROLLER), watch: Watch::Changed, required: false },
    Read { table: TableId::FactionMembers, columns: Columns::These(&FACTION), watch: Watch::Changed, required: false },
    Read { table: TableId::WarRelations, columns: Columns::These(&[]), watch: Watch::Changed, required: false },
    Read { table: TableId::Subjects, columns: Columns::These(&[]), watch: Watch::Changed, required: false },
    Read { table: TableId::RelationLinks, columns: Columns::These(&LINKS), watch: Watch::Changed, required: false },
];

const SELECTION: [TableId; 7] = [TableId::NavalBases, TableId::NavalRepairBases, TableId::NavalRepairQueue, TableId::Ships, TableId::ShipModels, TableId::TaskForces, TableId::Provinces];

fn invalid(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

fn rate_key(country: u32, part: u8, design: u64) -> u128 {
    u128::from(country) << 72 | u128::from(part) << 64 | u128::from(design)
}

fn rate_parts(key: u128) -> (u32, u8, u64) {
    ((key >> 72) as u32, (key >> 64) as u8, key as u64)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Selection {
    readable: bool,
    allocated: i64,
    occurrences: i64,
    designs: BTreeMap<u64, i64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Base {
    holder: Option<u32>,
    selection: Selection,
}

#[derive(Clone, Debug, Default)]
struct Queued {
    bases: Vec<u32>,
    force: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Written {
    country: u32,
    designs: Vec<u64>,
}

struct Lookups<'a, 'h> {
    hour: &'a Hour<'h>,
    queues: &'a HashMap<u32, Vec<(u32, u64)>>,
    producers: &'a Producers,
    relations: &'a Option<Result<(Relations, AccessLinks), &'static str>>,
}

impl Lookups<'_, '_> {
    fn complete(&self, table: TableId) -> Result<(), Unavailable> {
        if self.hour.complete(table) { Ok(()) } else { Err(Unavailable) }
    }
}

impl Operands for Lookups<'_, '_> {
    type Queue<'q>
        = std::iter::Copied<std::slice::Iter<'q, (u32, u64)>>
    where
        Self: 'q;

    fn allocation(&self, province: u32) -> Result<Option<i32>, Unavailable> {
        self.complete(TableId::NavalRepairBases)?;
        Ok(allocation(self.hour, province))
    }

    fn queue(&self, province: u32) -> Result<(Self::Queue<'_>, usize), Unavailable> {
        self.complete(TableId::NavalRepairQueue)?;
        let queue = self.queues.get(&province).map_or(&[][..], Vec::as_slice);
        Ok((queue.iter().copied(), queue.len()))
    }

    fn ship(&self, id: u64) -> Result<Option<QueuedShip>, Unavailable> {
        self.complete(TableId::Ships)?;
        ship(self.hour, id).map_err(|()| Unavailable)
    }

    fn force(&self, id: u64) -> Result<Option<QueuedForce>, Unavailable> {
        self.complete(TableId::TaskForces)?;
        force(self.hour, id).map_err(|()| Unavailable)
    }

    fn model(&self, ship: u64) -> Result<Option<QueuedModel>, Unavailable> {
        self.complete(TableId::ShipModels)?;
        model(self.hour, ship).map_err(|_| Unavailable)
    }

    fn controller(&self, province: u32) -> Result<Option<u32>, Unavailable> {
        self.complete(TableId::Provinces)?;
        controller(self.hour, province).map_err(|()| Unavailable)
    }

    fn slot_of(&self, raw: u32) -> Option<u32> {
        self.producers.slot(raw)
    }

    fn port(&self, owner: u32, controller: u32) -> Result<bool, &'static str> {
        match self.relations {
            Some(Ok((relations, access))) => Ok(relations.known_port(access, owner, controller)),
            Some(Err(why)) => Err(why),
            None => Err("txt_capacity: relations not read"),
        }
    }
}

fn selection_at(lookups: &Lookups<'_, '_>, province: u32) -> io::Result<Selection> {
    let hour = lookups.hour;
    let readable = hour.complete(TableId::NavalBases) && hour.complete(TableId::NavalRepairBases);
    let unreadable = Selection::default();
    let Some(base) = base(hour, province)? else { return Ok(unreadable) };
    let allocated = match lookups.allocation(province) {
        Ok(allocated) => allocated.unwrap_or(0),
        Err(Unavailable) => return Ok(unreadable),
    };
    let mut selection = Selection { readable, allocated: i64::from(allocated.max(0)), ..Selection::default() };
    if allocated <= 0 {
        return Ok(selection);
    }
    let Ok(selected) = naval_repairs::selected(lookups, &base) else { return Ok(unreadable) };
    selection.occurrences = selected.len() as i64;
    for occurrence in selected {
        if let Ok(Some(model)) = lookups.model(occurrence.ship)
            && model.variant != 0
        {
            *selection.designs.entry(model.variant).or_default() += 1;
        }
    }
    Ok(selection)
}

#[derive(Default)]
pub struct DockyardRepairs {
    rebuild: bool,
    producers: Producers,
    bases: HashMap<u32, Base>,
    holders: HashMap<u32, HashSet<u32>>,
    queues: HashMap<u32, Vec<(u32, u64)>>,
    queued: HashMap<u64, Queued>,
    forces: HashMap<u64, Vec<u64>>,
    relations: Option<Result<(Relations, AccessLinks), &'static str>>,
    written: HashMap<u32, Written>,
}

impl DockyardRepairs {
    fn lookups<'a, 'h>(&'a self, hour: &'a Hour<'h>) -> Lookups<'a, 'h> {
        Lookups { hour, queues: &self.queues, producers: &self.producers, relations: &self.relations }
    }

    fn enqueue(&mut self, hour: &Hour<'_>, ship: u64, province: u32) {
        if !self.queued.contains_key(&ship) {
            let force = self::ship(hour, ship).ok().flatten().map(|ship| ship.task_force);
            if let Some(force) = force {
                self.forces.entry(force).or_default().push(ship);
            }
            self.queued.insert(ship, Queued { bases: Vec::new(), force });
        }
        if let Some(queued) = self.queued.get_mut(&ship)
            && !queued.bases.contains(&province)
        {
            queued.bases.push(province);
        }
    }

    fn unqueue(&mut self, ship: u64, province: u32) {
        let Some(queued) = self.queued.get_mut(&ship) else { return };
        queued.bases.retain(|&held| held != province);
        if queued.bases.is_empty() {
            let force = queued.force;
            self.queued.remove(&ship);
            if let Some(force) = force {
                self.unforce(ship, force);
            }
        }
    }

    fn unforce(&mut self, ship: u64, force: u64) {
        if let Some(ships) = self.forces.get_mut(&force) {
            ships.retain(|&held| held != ship);
            if ships.is_empty() {
                self.forces.remove(&force);
            }
        }
    }

    fn set_queue(&mut self, hour: &Hour<'_>, province: u32, mut queue: Vec<(u32, u64)>) {
        queue.sort_unstable_by_key(|&(ordinal, _)| ordinal);
        for (_, ship) in self.queues.remove(&province).unwrap_or_default() {
            self.unqueue(ship, province);
        }
        for &(_, ship) in &queue {
            self.enqueue(hour, ship, province);
        }
        if !queue.is_empty() {
            self.queues.insert(province, queue);
        }
    }

    fn set_holder(&mut self, province: u32, holder: Option<Option<u32>>, dirty: &mut HashSet<u32>) {
        let old = self.bases.get(&province).map(|base| base.holder);
        if old == holder {
            return;
        }
        if let Some(Some(slot)) = old {
            dirty.insert(slot);
            if let Some(bases) = self.holders.get_mut(&slot) {
                bases.remove(&province);
                if bases.is_empty() {
                    self.holders.remove(&slot);
                }
            }
        }
        match holder {
            None => {
                self.bases.remove(&province);
            }
            Some(holder) => {
                if let Some(slot) = holder {
                    dirty.insert(slot);
                    self.holders.entry(slot).or_default().insert(province);
                }
                self.bases.entry(province).or_default().holder = holder;
            }
        }
    }

    fn read_whole(&mut self, hour: &Hour<'_>, out: &mut Rows) {
        self.producers.read(hour);
        self.relations = Some(relations(hour, &self.producers.links()));
        self.bases.clear();
        self.holders.clear();
        self.queues.clear();
        self.queued.clear();
        self.forces.clear();
        let queue = hour.table(TableId::NavalRepairQueue);
        let mut queues: HashMap<u32, Vec<(u32, u64)>> = HashMap::new();
        for slot in queue.rows() {
            let Some(key) = queue.key(slot) else { continue };
            let (province, ordinal) = queue_key(key);
            if let Some(ship) = queued(&queue, key) {
                queues.entry(province).or_default().push((ordinal, ship));
            }
        }
        for (province, entries) in queues {
            self.set_queue(hour, province, entries);
        }
        let bases = hour.table(TableId::NavalBases);
        let mut dirty = HashSet::new();
        for slot in bases.rows() {
            if let Some((province, holder)) = base_holder(&bases, slot) {
                let holder = self.producers.slot(holder);
                self.set_holder(province, Some(holder), &mut dirty);
            }
        }
        let slots: HashMap<u32, u32> = self.producers.holders().map(|(slot, country)| (country, slot)).collect();
        self.written.clear();
        let mut gone = Vec::new();
        for key in recorder_derive_framework::forces::held(out, TableId::DockyardRepairRates) {
            let (country, part, design) = rate_parts(key);
            match slots.get(&country) {
                Some(&slot) => {
                    let written = self.written.entry(slot).or_insert_with(|| Written { country, designs: Vec::new() });
                    if part == Rate::DESIGN {
                        written.designs.push(design);
                    }
                }
                None => gone.push(key),
            }
        }
        for key in gone {
            out.delete(TableId::DockyardRepairRates, key);
        }
    }

    fn apply(&mut self, hour: &Hour<'_>, bases: &mut HashSet<u32>, slots: &mut HashSet<u32>) -> io::Result<bool> {
        let mut all_bases = SELECTION.iter().any(|&table| hour.changes(table).status_changed);
        let mut all_slots = [TableId::NavalRepairCountries, TableId::CountryProductionModifiers].iter().any(|&table| hour.changes(table).status_changed);
        if Producers::moved(hour) {
            self.producers.read(hour);
            all_slots = true;
            all_bases = true;
            let view = hour.table(TableId::NavalBases);
            let provinces: Vec<u32> = self.bases.keys().copied().collect();
            for province in provinces {
                let holder = view.slot(u128::from(province)).and_then(|slot| base_holder(&view, slot)).and_then(|(_, raw)| self.producers.slot(raw));
                self.set_holder(province, Some(holder), slots);
            }
        }
        if all_slots || relations_moved(hour) {
            self.relations = Some(relations(hour, &self.producers.links()));
            all_bases = true;
        }
        let table = TableId::NavalRepairQueue;
        let mut provinces: HashSet<u32> = hour.deleted_keys(table).map(|key| queue_key(key).0).collect();
        let view = hour.table(table);
        let changed: Vec<u128> = hour.deleted_keys(table).chain(hour.written_rows(table, Columns::These(&QUEUE)).into_iter().filter_map(|slot| view.key(slot))).collect();
        provinces.extend(changed.iter().map(|&key| queue_key(key).0));
        for &province in &provinces {
            let mut queue = self.queues.get(&province).cloned().unwrap_or_default();
            for &key in changed.iter().filter(|&&key| queue_key(key).0 == province) {
                let ordinal = queue_key(key).1;
                queue.retain(|&(held, _)| held != ordinal);
                if let Some(ship) = queued(&view, key) {
                    queue.push((ordinal, ship));
                }
            }
            self.set_queue(hour, province, queue);
            bases.insert(province);
        }
        let table = TableId::NavalBases;
        let view = hour.table(table);
        for key in hour.deleted_keys(table) {
            self.set_holder(key as u32, None, slots);
        }
        for slot in hour.written_rows(table, Columns::These(&BASE)) {
            if let Some((province, raw)) = base_holder(&view, slot) {
                self.set_holder(province, Some(self.producers.slot(raw)), slots);
                bases.insert(province);
            }
        }
        for table in [TableId::NavalRepairBases, TableId::Provinces] {
            let view = hour.table(table);
            let columns: &'static [usize] = if table == TableId::Provinces { &CONTROLLER } else { &ALLOCATION };
            bases.extend(hour.deleted_keys(table).map(|key| key as u32));
            bases.extend(hour.written_rows(table, Columns::These(columns)).into_iter().filter_map(|slot| view.key(slot)).map(|key| key as u32));
        }
        let mut ships: Vec<u64> = Vec::new();
        for (table, columns) in [(TableId::Ships, &SHIP as &'static [usize]), (TableId::ShipModels, &MODEL)] {
            let view = hour.table(table);
            ships.extend(hour.deleted_keys(table).map(|key| key as u64));
            ships.extend(hour.written_rows(table, Columns::These(columns)).into_iter().filter_map(|slot| view.key(slot)).map(|key| key as u64));
        }
        let view = hour.table(TableId::TaskForces);
        let forces: Vec<u64> = hour
            .deleted_keys(TableId::TaskForces)
            .chain(hour.written_rows(TableId::TaskForces, Columns::These(&FORCE)).into_iter().filter_map(|slot| view.key(slot)))
            .map(|key| key as u64)
            .collect();
        for force in forces {
            ships.extend(self.forces.get(&force).into_iter().flatten().copied());
        }
        ships.sort_unstable();
        ships.dedup();
        for ship in ships {
            let Some(queued) = self.queued.get_mut(&ship) else { continue };
            bases.extend(queued.bases.iter().copied());
            let force = self::ship(hour, ship).ok().flatten().map(|ship| ship.task_force);
            if force != queued.force {
                let old = std::mem::replace(&mut queued.force, force);
                if let Some(old) = old {
                    self.unforce(ship, old);
                }
                if let Some(force) = force {
                    self.forces.entry(force).or_default().push(ship);
                }
            }
        }
        for table in [TableId::NavalRepairCountries, TableId::CountryProductionModifiers] {
            let view = hour.table(table);
            let columns: &'static [usize] = if table == TableId::NavalRepairCountries { &COUNT } else { &MODIFIERS };
            let keys: Vec<u128> = hour.deleted_keys(table).chain(hour.written_rows(table, Columns::These(columns)).into_iter().filter_map(|slot| view.key(slot))).collect();
            slots.extend(keys.into_iter().filter_map(|key| self.producers.slot(key as u32)));
        }
        if all_bases {
            bases.extend(self.bases.keys().copied());
        }
        Ok(all_slots)
    }

    fn select(&mut self, hour: &Hour<'_>, bases: HashSet<u32>, slots: &mut HashSet<u32>) -> io::Result<()> {
        let mut moved = Vec::new();
        {
            let lookups = self.lookups(hour);
            for province in bases {
                let Some(base) = self.bases.get(&province) else { continue };
                let selection = selection_at(&lookups, province)?;
                if selection != base.selection {
                    slots.extend(base.holder);
                    moved.push((province, selection));
                }
            }
        }
        for (province, selection) in moved {
            if let Some(base) = self.bases.get_mut(&province) {
                base.selection = selection;
            }
        }
        Ok(())
    }

    fn unique(&self, slot: u32, has: impl Fn(u32) -> bool) -> Option<u32> {
        let mut found = None;
        for &raw in self.producers.aliases(slot) {
            if has(raw) {
                if found.is_some() {
                    return None;
                }
                found = Some(raw);
            }
        }
        found
    }

    fn write(&mut self, hour: &Hour<'_>, out: &mut Rows, slot: u32) -> io::Result<()> {
        let table = TableId::DockyardRepairRates;
        let country = self.producers.of_slot(slot);
        let dockyards = country.and(self.unique(slot, |raw| has_dockyards(hour, raw)));
        let (Some(country), Some(raw)) = (country, dockyards) else {
            if let Some(written) = self.written.remove(&slot) {
                out.delete(table, rate_key(written.country, Rate::TOTAL, 0));
                out.delete(table, rate_key(written.country, Rate::UNATTRIBUTED, 0));
                for design in written.designs {
                    out.delete(table, rate_key(written.country, Rate::DESIGN, design));
                }
            }
            return Ok(());
        };
        let count = if hour.complete(TableId::NavalRepairCountries) { used_dockyards(hour, raw).filter(|&count| count >= 0) } else { None };
        let output = match (hour.complete(TableId::CountryProductionModifiers), self.unique(slot, |raw| has_modifiers(hour, raw))) {
            (true, Some(raw)) => dockyard_output(hour, raw)?,
            _ => None,
        };
        let times = |ships: i64| output.and_then(|output| ships.checked_mul(output));
        let rate = match count {
            Some(0) => Some(0),
            Some(count) => times(i64::from(count)),
            None => None,
        };
        let mut selection = Selection { readable: hour.complete(TableId::NavalBases) && hour.complete(TableId::NavalRepairBases), ..Selection::default() };
        for province in self.holders.get(&slot).into_iter().flatten() {
            let held = &self.bases[province].selection;
            selection.readable &= held.readable;
            selection.allocated += held.allocated;
            selection.occurrences += held.occurrences;
            for (&design, &ships) in &held.designs {
                *selection.designs.entry(design).or_default() += ships;
            }
        }
        let reconciled = match count {
            Some(0) => true,
            Some(count) => selection.readable && selection.allocated == i64::from(count) && selection.occurrences <= i64::from(count),
            None => false,
        };
        let split = rate.is_some() && reconciled;
        let cell = |value: Option<i64>| value.map_or(Value::Unrecorded, Value::I64);
        out.set(table, rate_key(country, Rate::TOTAL, 0), vec![cell(rate), count.map_or(Value::Unrecorded, Value::I32), cell(output), Value::Bool(split)]);
        let mut designs = Vec::new();
        let unattributed = match (split, count) {
            (false, _) => None,
            (true, Some(0)) => Some(0),
            (true, _) => {
                let mut attributed = 0_i64;
                for (&design, &ships) in &selection.designs {
                    attributed += ships;
                    designs.push(design);
                    out.set(table, rate_key(country, Rate::DESIGN, design), vec![cell(times(ships)), Value::Unrecorded, Value::Unrecorded, Value::Unrecorded]);
                }
                times(i64::from(count.unwrap_or(0)) - attributed)
            }
        };
        out.set(table, rate_key(country, Rate::UNATTRIBUTED, 0), vec![cell(unattributed), Value::Unrecorded, Value::Unrecorded, Value::Unrecorded]);
        let written = Written { country, designs };
        if let Some(old) = self.written.insert(slot, written.clone()) {
            if old.country != country {
                out.delete(table, rate_key(old.country, Rate::TOTAL, 0));
                out.delete(table, rate_key(old.country, Rate::UNATTRIBUTED, 0));
            }
            for design in old.designs {
                if old.country != country || !written.designs.contains(&design) {
                    out.delete(table, rate_key(old.country, Rate::DESIGN, design));
                }
            }
        }
        Ok(())
    }
}

impl Writer for DockyardRepairs {
    fn name(&self) -> &'static str {
        "DockyardRepairs"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::DockyardRepairRates)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let whole = hour.fresh || self.rebuild;
        if !whole && !READS.iter().any(|read| hour.changes(read.table).status_changed || hour.wrote(read.table, read.columns)) {
            return Ok(());
        }
        let mut bases = HashSet::new();
        let mut slots = HashSet::new();
        let all = match whole {
            true => {
                self.read_whole(hour, out);
                self.rebuild = false;
                bases.extend(self.bases.keys().copied());
                true
            }
            false => self.apply(hour, &mut bases, &mut slots)?,
        };
        self.select(hour, bases, &mut slots)?;
        if all {
            slots.extend(self.producers.holders().map(|(slot, _)| slot));
            slots.extend(self.written.keys().copied());
        }
        let mut slots: Vec<u32> = slots.into_iter().collect();
        slots.sort_unstable();
        for slot in slots {
            self.write(hour, out, slot)?;
        }
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("DockyardRepairs' state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}
