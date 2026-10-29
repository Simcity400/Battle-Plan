use replay::{columns, segments};
use replay_data::rows::TableId;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
};

/// A cell the recording did not record, which a row decoded from the columns refuses too.
pub(super) fn unrecorded(table: TableId, slot: u32) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{table:?} slot {slot} holds no recorded value"),
    )
}

/// Notes `owner` once for a run of slots it holds, as an owner's rows mostly are.
fn touch(touched: &mut Vec<u32>, owner: u32) {
    if touched.last() != Some(&owner) {
        touched.push(owner);
    }
}

/// A table's live slots by the owner their keys name, each owner's in key order.
pub(super) struct Groups {
    owner: fn(u128) -> u32,
    complete: bool,
    slots: HashMap<u32, Vec<u32>>,
    /// The owners whose rows the last hour may have changed, ascending.
    pub(super) touched: Vec<u32>,
}
impl Groups {
    pub(super) fn new(owner: fn(u128) -> u32) -> Self {
        Self {
            owner,
            complete: false,
            slots: HashMap::new(),
            touched: Vec::new(),
        }
    }
    /// Follows the hour's changes to `table`; empty while it is not complete.
    pub(super) fn update(&mut self, hour: &segments::Hour<'_>, table: TableId) -> io::Result<()> {
        self.touched.clear();
        let complete = columns::complete(hour.state, table);
        let whole = hour.starts || hour.opens || !self.complete;
        self.complete = complete;
        if whole || !complete {
            self.touched
                .extend(self.slots.drain().map(|(owner, _)| owner));
        }
        if complete {
            let held = columns::table(hour.state, table)?;
            if whole {
                for slot in held.live().ones() {
                    if let Some(key) = held.key(slot) {
                        self.slots.entry((self.owner)(key)).or_default().push(slot);
                    }
                }
                for (&owner, slots) in &mut self.slots {
                    slots.sort_unstable_by_key(|&slot| held.key(slot));
                    self.touched.push(owner);
                }
            } else {
                let edits = columns::changes(hour.state, table)?;
                for (&slot, &key) in edits.deleted.iter().zip(&edits.deleted_keys) {
                    let owner = (self.owner)(key);
                    if let Some(slots) = self.slots.get_mut(&owner) {
                        slots.retain(|&other| other != slot);
                        if slots.is_empty() {
                            self.slots.remove(&owner);
                        }
                    }
                    touch(&mut self.touched, owner);
                }
                for &slot in &edits.inserted {
                    if let Some(key) = held.key(slot) {
                        let owner = (self.owner)(key);
                        let slots = self.slots.entry(owner).or_default();
                        let at = slots.partition_point(|&other| held.key(other) < Some(key));
                        slots.insert(at, slot);
                        touch(&mut self.touched, owner);
                    }
                }
                for &slot in edits.changed.iter().chain(&edits.fill_slots) {
                    if let Some(key) = held.key(slot) {
                        touch(&mut self.touched, (self.owner)(key));
                    }
                }
            }
        }
        self.touched.sort_unstable();
        self.touched.dedup();
        Ok(())
    }
    pub(super) fn owner(&self, owner: u32) -> &[u32] {
        self.slots
            .get(&owner)
            .map(Vec::as_slice)
            .unwrap_or_default()
    }
    pub(super) fn owners(&self) -> impl Iterator<Item = (u32, usize)> + '_ {
        self.slots
            .iter()
            .map(|(&owner, slots)| (owner, slots.len()))
    }
}

const STAMPED: [TableId; 2] = [TableId::RelationOpinions, TableId::RelationTuples];
fn stamped(table: u32) -> Option<usize> {
    STAMPED.iter().position(|kept| kept.index() as u32 == table)
}

/// SliceReads' stamps of `STAMPED`, by owner and by read hour.
#[derive(Default)]
pub(super) struct Stamps {
    complete: bool,
    reads: [HashMap<u32, i32>; 2],
    hours: [BTreeMap<i32, BTreeSet<u32>>; 2],
}
impl Stamps {
    pub(super) fn update(&mut self, hour: &segments::Hour<'_>, field: usize) -> io::Result<()> {
        let table = TableId::SliceReads;
        let complete = columns::complete(hour.state, table);
        let whole = hour.starts || hour.opens || !self.complete;
        self.complete = complete;
        if whole || !complete {
            for (reads, hours) in self.reads.iter_mut().zip(&mut self.hours) {
                reads.clear();
                hours.clear();
            }
        }
        if !complete {
            return Ok(());
        }
        let held = columns::table(hour.state, table)?;
        let cells = columns::plain::<i32>(held, field)?;
        let decode = |slot: u32| -> io::Result<Option<(usize, u32, i32)>> {
            let Some((at, owner)) = held
                .key(slot)
                .and_then(|key| Some((stamped((key >> 32) as u32)?, key as u32)))
            else {
                return Ok(None);
            };
            match cells.get(slot as usize) {
                Some(&read) if held.recorded(field, slot) => Ok(Some((at, owner, read))),
                _ => Err(unrecorded(table, slot)),
            }
        };
        if whole {
            for slot in held.live().ones() {
                if let Some((at, owner, read)) = decode(slot)? {
                    self.set(at, owner, read);
                }
            }
            return Ok(());
        }
        let edits = columns::changes(hour.state, table)?;
        for &key in &edits.deleted_keys {
            if let Some(at) = stamped((key >> 32) as u32) {
                self.remove(at, key as u32);
            }
        }
        for &slot in edits
            .changed
            .iter()
            .chain(&edits.fill_slots)
            .chain(&edits.inserted)
        {
            if let Some((at, owner, read)) = decode(slot)? {
                self.set(at, owner, read);
            }
        }
        Ok(())
    }
    fn set(&mut self, at: usize, owner: u32, read: i32) {
        if let Some(old) = self.reads[at].insert(owner, read) {
            self.forget(at, owner, old);
        }
        self.hours[at].entry(read).or_default().insert(owner);
    }
    fn remove(&mut self, at: usize, owner: u32) {
        if let Some(old) = self.reads[at].remove(&owner) {
            self.forget(at, owner, old);
        }
    }
    fn forget(&mut self, at: usize, owner: u32, read: i32) {
        if let Some(owners) = self.hours[at].get_mut(&read) {
            owners.remove(&owner);
            if owners.is_empty() {
                self.hours[at].remove(&read);
            }
        }
    }
    pub(super) fn stamp(&self, table: TableId, owner: u32) -> Option<i32> {
        self.reads[stamped(table.index() as u32)?]
            .get(&owner)
            .copied()
    }
    pub(super) fn current(&self, table: TableId, hour: i32) -> impl Iterator<Item = u32> + '_ {
        stamped(table.index() as u32)
            .and_then(|at| self.hours[at].get(&hour))
            .into_iter()
            .flatten()
            .copied()
    }
}
