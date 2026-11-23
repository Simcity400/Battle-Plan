
use recorder_derive_framework::{
    forces::{invalid, written},
    framework::{Columns, Hour, Output, Read, Rows, Value, Watch, Writer, columns},
};
use replay_columnar::{leaf::Writer as Bytes, store::TableView};
use replay_data::rows::{AirTheatreGroupRow, AirTheatreMemberRow, AirWingRow, TableId};
use std::{
    collections::{HashMap, HashSet},
    io,
};

const REGION: [usize; 1] = columns::<AirWingRow, 1>(["mission_region"]);
const THEATRE: [usize; 1] = columns::<AirTheatreGroupRow, 1>(["theatre"]);
const ORDINAL: [usize; 1] = columns::<AirTheatreMemberRow, 1>(["ordinal"]);

const READS: &[Read] = &[
    Read { table: TableId::AirTheatreMembers, columns: Columns::These(&ORDINAL), watch: Watch::Changed, required: true },
    Read { table: TableId::AirTheatreGroups, columns: Columns::These(&THEATRE), watch: Watch::Changed, required: true },
    Read { table: TableId::AirWings, columns: Columns::These(&REGION), watch: Watch::Changed, required: true },
];

pub fn key(theatre: u64, region: u32) -> u128 {
    u128::from(theatre) << 32 | u128::from(region)
}

fn region_of(wings: &TableView<'_>, wing: u64) -> io::Result<Option<u32>> {
    let Some(slot) = wings.slot(u128::from(wing)) else { return Ok(None) };
    match wings.value(REGION[0], slot) {
        replay_columnar::store::Value::Number(region) => Ok(Some(region as u32)),
        replay_columnar::store::Value::Missing => Ok(None),
        _ => Err(invalid("AirWings.mission_region is not recorded and its row cannot say so")),
    }
}

#[derive(Default)]
pub struct AirTheatreRegions {
    rebuild: bool,
    groups: HashMap<u64, u64>,
    members: HashMap<u64, HashSet<u64>>,
    theatres: HashMap<u64, u64>,
    regions: HashMap<u64, u32>,
    placed: HashMap<u64, (u64, u32)>,
    counts: HashMap<(u64, u32), i64>,
    dirty: HashSet<(u64, u32)>,
}

impl AirTheatreRegions {
    fn count(&mut self, place: (u64, u32), sign: i64) {
        let count = self.counts.entry(place).or_default();
        *count += sign;
        if *count == 0 {
            self.counts.remove(&place);
        }
        self.dirty.insert(place);
    }

    fn place(&mut self, wing: u64) {
        let place = self.groups.get(&wing).and_then(|group| self.theatres.get(group)).copied().zip(self.regions.get(&wing).copied());
        let before = match place {
            Some(place) => self.placed.insert(wing, place),
            None => self.placed.remove(&wing),
        };
        if before == place {
            return;
        }
        if let Some(before) = before {
            self.count(before, -1);
        }
        if let Some(place) = place {
            self.count(place, 1);
        }
    }

    fn join(&mut self, wing: u64, group: u64, region: Option<u32>) {
        if let Some(old) = self.groups.insert(wing, group).filter(|old| *old != group)
            && let Some(wings) = self.members.get_mut(&old)
        {
            wings.remove(&wing);
        }
        self.members.entry(group).or_default().insert(wing);
        match region {
            Some(region) => self.regions.insert(wing, region),
            None => self.regions.remove(&wing),
        };
        self.place(wing);
    }

    fn leave(&mut self, wing: u64, group: u64) {
        if self.groups.get(&wing) != Some(&group) {
            return;
        }
        self.groups.remove(&wing);
        self.regions.remove(&wing);
        if let Some(wings) = self.members.get_mut(&group) {
            wings.remove(&wing);
        }
        self.place(wing);
    }

    fn regroup(&mut self, group: u64, theatre: Option<u64>) {
        match theatre {
            Some(theatre) => self.theatres.insert(group, theatre),
            None => self.theatres.remove(&group),
        };
        let wings: Vec<u64> = self.members.get(&group).into_iter().flatten().copied().collect();
        for wing in wings {
            self.place(wing);
        }
    }
}

fn member(view: &TableView<'_>, slot: u32) -> io::Result<(u64, u64)> {
    let key = view.key(slot).ok_or_else(|| invalid(format!("AirTheatreMembers slot {slot} holds no row")))?;
    Ok(((key >> 64) as u64, key as u64))
}

fn group(view: &TableView<'_>, slot: u32) -> io::Result<(u64, u64)> {
    let id = view.key(slot).ok_or_else(|| invalid(format!("AirTheatreGroups slot {slot} holds no row")))? as u64;
    let theatre = view.number(THEATRE[0], slot).ok_or_else(|| invalid("AirTheatreGroups.theatre is not recorded and its row cannot say so"))?;
    Ok((id, theatre as u64))
}

impl Writer for AirTheatreRegions {
    fn name(&self) -> &'static str {
        "AirTheatreRegions"
    }

    fn version(&self) -> u32 {
        1
    }

    fn tables(&self) -> &'static [Output] {
        &[Output::State(TableId::AirTheatreRegions)]
    }

    fn reads(&self) -> &'static [Read] {
        READS
    }

    fn start(&mut self) {
        *self = Self::default();
    }

    fn hour(&mut self, hour: &Hour<'_>, out: &mut Rows) -> io::Result<()> {
        let table = TableId::AirTheatreRegions;
        let whole = hour.fresh || self.rebuild;
        let (members, groups, wings) = (hour.table(TableId::AirTheatreMembers), hour.table(TableId::AirTheatreGroups), hour.table(TableId::AirWings));
        if whole {
            let held: Vec<u128> = recorder_derive_framework::forces::held(out, table);
            *self = Self::default();
            for slot in groups.rows() {
                let (id, theatre) = group(&groups, slot)?;
                self.theatres.insert(id, theatre);
            }
            for slot in members.rows() {
                let (group, wing) = member(&members, slot)?;
                self.join(wing, group, region_of(&wings, wing)?);
            }
            self.dirty.extend(held.into_iter().map(|key| ((key >> 32) as u64, key as u32)));
        } else {
            for key in hour.deleted_keys(TableId::AirTheatreGroups) {
                self.regroup(key as u64, None);
            }
            for slot in written(hour, TableId::AirTheatreGroups, &THEATRE) {
                let (id, theatre) = group(&groups, slot)?;
                self.regroup(id, Some(theatre));
            }
            for key in hour.deleted_keys(TableId::AirTheatreMembers) {
                self.leave(key as u64, (key >> 64) as u64);
            }
            for slot in written(hour, TableId::AirTheatreMembers, &ORDINAL) {
                let (group, wing) = member(&members, slot)?;
                self.join(wing, group, region_of(&wings, wing)?);
            }
            for key in hour.deleted_keys(TableId::AirWings) {
                if self.regions.remove(&(key as u64)).is_some() {
                    self.place(key as u64);
                }
            }
            for slot in written(hour, TableId::AirWings, &REGION) {
                let Some(wing) = wings.key(slot).map(|key| key as u64) else { continue };
                if self.groups.contains_key(&wing) {
                    match region_of(&wings, wing)? {
                        Some(region) => self.regions.insert(wing, region),
                        None => self.regions.remove(&wing),
                    };
                    self.place(wing);
                }
            }
        }
        let mut dirty: Vec<(u64, u32)> = std::mem::take(&mut self.dirty).into_iter().collect();
        dirty.sort_unstable();
        for place in dirty {
            let row = key(place.0, place.1);
            match self.counts.get(&place) {
                Some(&wings) => out.set(table, row, vec![Value::I64(wings)]),
                None if out.table(table).slot(row).is_some() => out.delete(table, row),
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
            return Err(invalid("AirTheatreRegions' state holds nothing"));
        }
        self.rebuild = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
