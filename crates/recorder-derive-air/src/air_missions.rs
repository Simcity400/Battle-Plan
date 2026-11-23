
use recorder_derive_framework::{
    forces::{Owners, invalid, links_moved, number, written},
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns},
};
use replay_columnar::{
    leaf::Writer as Bytes,
    manifest::Status,
    store::{TableView, Value as Held},
};
use replay_data::rows::{AirMissionCountRow, AirRegionalMissionRow, AirWingRow, TableId};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    io,
};

const WING_COLUMNS: [usize; 4] = columns::<AirWingRow, 4>(["owner", "mission_region", "mission_mask", "planes"]);
const REGISTRATION_COLUMNS: [usize; 1] = columns::<AirRegionalMissionRow, 1>(["ordinal"]);

const READS: &[Read] = &[
    Read { table: TableId::AirRegionalMissions, columns: Columns::These(&REGISTRATION_COLUMNS), watch: Watch::Changed, required: true },
    Read { table: TableId::AirWings, columns: Columns::These(&WING_COLUMNS), watch: Watch::Changed, required: true },
    Read { table: TableId::CountryLinks, columns: Columns::These(&[0]), watch: Watch::Changed, required: true },
];

const NO_WING: usize = 0;
const UNRESOLVED: usize = 1;
const NO_REGION: usize = 2;
const REASONS: [&str; 3] = [
    "AirMissions: a registered wing has no AirWings row",
    "AirMissions: a registered wing's owner resolves to no country",
    "AirMissions: a registered wing has no mission region",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Place {
    Group(u32, u32),
    Broken(usize),
}

fn group_key((slot, region): (u32, u32)) -> u128 {
    u128::from(slot) << 32 | u128::from(region)
}

fn wing_key((slot, region): (u32, u32), wing: u64) -> u128 {
    u128::from(slot) << 96 | u128::from(region) << 64 | u128::from(wing)
}

fn group_row(members: impl ExactSizeIterator<Item = (u32, i32)>) -> Vec<Value> {
    let wings = members.len() as i64;
    let mut totals = [-1; AirMissionCountRow::MISSIONS];
    let mut selected = 0u32;
    for (mask, planes) in members {
        AirMissionCountRow::count(&mut totals, mask, planes);
        if mask as i32 > 0 {
            selected |= mask;
        }
    }
    let mut cells = Vec::with_capacity(1 + AirMissionCountRow::MISSIONS);
    cells.push(Value::I64(wings));
    cells.extend(totals.iter().enumerate().map(|(bit, &total)| if selected & (1 << bit) != 0 { Value::I32(total) } else { Value::Unrecorded }));
    cells
}

#[derive(Default)]
pub struct AirMissions {
    rebuild: bool,
    owners: Owners,
    registered: HashMap<u64, (u32, Place)>,
    groups: HashMap<(u32, u32), BTreeSet<(u32, u64)>>,
    broken: BTreeMap<u64, usize>,
    dirty: HashSet<(u32, u32)>,
    placed: HashSet<u64>,
    left: Vec<u128>,
}

impl AirMissions {
    fn place(&self, wings: &TableView<'_>, wing: u64) -> io::Result<Place> {
        let Some(slot) = wings.slot(u128::from(wing)) else { return Ok(Place::Broken(NO_WING)) };
        let Some(owner) = self.owners.slot(number(wings, TableId::AirWings, WING_COLUMNS[0], slot)? as u32) else {
            return Ok(Place::Broken(UNRESOLVED));
        };
        match wings.value(WING_COLUMNS[1], slot) {
            Held::Number(region) => Ok(Place::Group(owner, region as u32)),
            Held::Missing => Ok(Place::Broken(NO_REGION)),
            _ => Err(invalid("AirWings.mission_region is not recorded and its row cannot say so")),
        }
    }

    fn refresh(&mut self, wings: &TableView<'_>, wing: u64, ordinal: Option<u32>) -> io::Result<()> {
        if let Some((ordinal, place)) = self.registered.remove(&wing) {
            match place {
                Place::Group(slot, region) => {
                    if let Some(members) = self.groups.get_mut(&(slot, region)) {
                        members.remove(&(ordinal, wing));
                        if members.is_empty() {
                            self.groups.remove(&(slot, region));
                        }
                    }
                    self.dirty.insert((slot, region));
                    self.left.push(wing_key((slot, region), wing));
                }
                Place::Broken(_) => {
                    self.broken.remove(&wing);
                }
            }
        }
        let Some(ordinal) = ordinal else { return Ok(()) };
        let place = self.place(wings, wing)?;
        match place {
            Place::Group(slot, region) => {
                self.groups.entry((slot, region)).or_default().insert((ordinal, wing));
                self.dirty.insert((slot, region));
                self.placed.insert(wing);
            }
            Place::Broken(reason) => {
                self.broken.insert(wing, reason);
            }
        }
        self.registered.insert(wing, (ordinal, place));
        Ok(())
    }

    fn read_all(&mut self, hour: &Hour<'_>) -> io::Result<()> {
        self.owners.read_links(hour);
        self.registered.clear();
        self.groups.clear();
        self.broken.clear();
        self.placed.clear();
        self.left.clear();
        let (registrations, wings) = (hour.table(TableId::AirRegionalMissions), hour.table(TableId::AirWings));
        for slot in registrations.rows() {
            let wing = registrations.key(slot).ok_or_else(|| invalid(format!("AirRegionalMissions slot {slot} holds no row")))? as u64;
            let ordinal = number(&registrations, TableId::AirRegionalMissions, REGISTRATION_COLUMNS[0], slot)? as u32;
            self.refresh(&wings, wing, Some(ordinal))?;
        }
        Ok(())
    }

    fn apply(&mut self, hour: &Hour<'_>) -> io::Result<()> {
        let (registrations, wings) = (hour.table(TableId::AirRegionalMissions), hour.table(TableId::AirWings));
        for key in hour.deleted_keys(TableId::AirRegionalMissions) {
            self.refresh(&wings, key as u64, None)?;
        }
        for slot in written(hour, TableId::AirRegionalMissions, &REGISTRATION_COLUMNS) {
            let wing = registrations.key(slot).ok_or_else(|| invalid(format!("AirRegionalMissions slot {slot} holds no row")))? as u64;
            let ordinal = number(&registrations, TableId::AirRegionalMissions, REGISTRATION_COLUMNS[0], slot)? as u32;
            self.refresh(&wings, wing, Some(ordinal))?;
        }
        let mut moved: Vec<u64> = hour.deleted_keys(TableId::AirWings).map(|key| key as u64).collect();
        moved.extend(written(hour, TableId::AirWings, &WING_COLUMNS).into_iter().filter_map(|slot| wings.key(slot)).map(|key| key as u64));
        for wing in moved {
            if let Some(&(ordinal, _)) = self.registered.get(&wing) {
                self.refresh(&wings, wing, Some(ordinal))?;
            }
        }
        Ok(())
    }

    fn row(&self, wings: &TableView<'_>, group: (u32, u32)) -> io::Result<Option<Vec<Value>>> {
        let Some(members) = self.groups.get(&group) else { return Ok(None) };
        let mut read = Vec::with_capacity(members.len());
        for &(_, wing) in members {
            let slot = wings.slot(u128::from(wing)).ok_or_else(|| invalid(format!("A counted wing {wing} has no AirWings row")))?;
            let mask = number(wings, TableId::AirWings, WING_COLUMNS[2], slot)? as u32;
            let planes = number(wings, TableId::AirWings, WING_COLUMNS[3], slot)? as i32;
            read.push((mask, planes));
        }
        Ok(Some(group_row(read.into_iter())))
    }

    fn write_wings(&mut self, out: &mut Rows<'_>, all: bool) {
        let table = TableId::AirMissionWings;
        let placed = std::mem::take(&mut self.placed);
        let mut left = std::mem::take(&mut self.left);
        let mut rows: HashMap<u128, u32> = HashMap::new();
        let mut row = |wing: u64, registered: Option<&(u32, Place)>| {
            if let Some(&(ordinal, Place::Group(slot, region))) = registered {
                rows.insert(wing_key((slot, region), wing), ordinal);
            }
        };
        if all {
            self.registered.iter().for_each(|(&wing, registered)| row(wing, Some(registered)));
            left = recorder_derive_framework::forces::held(out, table);
        } else {
            placed.into_iter().for_each(|wing| row(wing, self.registered.get(&wing)));
        }
        left.sort_unstable();
        left.dedup();
        for key in left {
            if !rows.contains_key(&key) && out.table(table).slot(key).is_some() {
                out.delete(table, key);
            }
        }
        let mut rows: Vec<(u128, u32)> = rows.into_iter().collect();
        rows.sort_unstable();
        for (key, ordinal) in rows {
            out.set(table, key, vec![Value::U32(ordinal)]);
        }
    }
}

impl Writer for AirMissions {
    fn name(&self) -> &'static str {
        "AirMissions"
    }

    fn version(&self) -> u32 {
        2
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::AirMissionCounts), Output::State(TableId::AirMissionWings)]
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
        let table = TableId::AirMissionCounts;
        let whole = hour.fresh || self.rebuild || links_moved(hour);
        let withheld = !hour.first && out.table(table).status() != Status::Complete;
        let wings_withheld = !hour.first && out.table(TableId::AirMissionWings).status() != Status::Complete;
        if whole {
            self.read_all(hour)?;
            self.rebuild = false;
        } else {
            self.apply(hour)?;
        }
        if let Some(&reason) = self.broken.values().next() {
            self.dirty.clear();
            self.placed.clear();
            self.left.clear();
            out.status(table, Status::Unavailable { reason: hour.strings[reason] });
            out.status(TableId::AirMissionWings, Status::Unavailable { reason: hour.strings[reason] });
            return Ok(());
        }
        self.write_wings(out, whole || wings_withheld);
        let dirty = std::mem::take(&mut self.dirty);
        let mut keys: Vec<(u32, u32)> = match whole || withheld {
            true => {
                let mut keys: Vec<(u32, u32)> = self.groups.keys().copied().collect();
                keys.extend(recorder_derive_framework::forces::held(out, table).into_iter().map(|key| ((key >> 32) as u32, key as u32)));
                keys
            }
            false => dirty.into_iter().collect(),
        };
        keys.sort_unstable();
        keys.dedup();
        let wings = hour.table(TableId::AirWings);
        for group in keys {
            let key = group_key(group);
            match self.row(&wings, group)? {
                Some(cells) => out.set(table, key, cells),
                None if out.table(table).slot(key).is_some() => out.delete(table, key),
                None => {}
            }
        }
        Ok(())
    }

    fn save(&self, _blob: &mut Bytes) -> io::Result<()> {
        Ok(())
    }

    fn load(&mut self, blob: &[u8]) -> io::Result<()> {
        if !blob.is_empty() {
            return Err(invalid("AirMissions' state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
