//! Each state's owner, and the states each canonical slot owns, kept from the hour's written
//! owners and moved links.

use super::{Facts, Roster, countries::edited, invalid};
use replay::{columns, segments::State};
use replay_data::{
    format::FormatVersion,
    rows::{StateRow, TableId},
};
use std::{collections::HashMap, io};

/// The states' owners of the hour, held only while States is complete.
pub struct Owners {
    field: usize,
    complete: bool,
    /// Each state's owner, a raw link id.
    owners: HashMap<u32, u32>,
    /// Each owning raw link id's states, and the canonical slot its link names.
    raws: HashMap<u32, (u32, Option<u32>)>,
    /// Each owning canonical slot's states.
    counts: HashMap<u32, u32>,
    /// States whose owner no link names.
    unresolved: u32,
    rebuilt: bool,
    moved: Vec<u32>,
    reached: Vec<u32>,
    /// Whether each slot whose count the hour changed owned states before it.
    owning: HashMap<u32, bool>,
    #[cfg(test)]
    pub(crate) reads: usize,
}

impl Owners {
    pub(crate) fn new() -> io::Result<Self> {
        Ok(Self {
            field: columns::field(TableId::States, "owner")?,
            complete: false,
            owners: HashMap::new(),
            raws: HashMap::new(),
            counts: HashMap::new(),
            unresolved: 0,
            rebuilt: false,
            moved: Vec::new(),
            reached: Vec::new(),
            owning: HashMap::new(),
            #[cfg(test)]
            reads: 0,
        })
    }

    /// Whether States holds every row this hour.
    pub fn complete(&self) -> bool {
        self.complete
    }

    /// State `id`'s owner, a raw link id.
    pub fn owner(&self, id: u32) -> Option<u32> {
        self.owners.get(&id).copied()
    }

    /// The states canonical slot `slot`'s links own.
    pub fn owned(&self, slot: u32) -> u32 {
        self.counts.get(&slot).copied().unwrap_or(0)
    }

    /// The states whose owner no link names.
    pub fn unresolved(&self) -> u32 {
        self.unresolved
    }

    /// Of those, the states whose owner is not raw id 0, which names no owner unless a link
    /// holds it.
    pub fn unlinked(&self) -> u32 {
        let unowned = self
            .raws
            .get(&0)
            .filter(|(_, slot)| slot.is_none())
            .map_or(0, |&(states, _)| states);
        self.unresolved - unowned
    }

    /// Every canonical slot owning states, in no order.
    pub fn slots(&self) -> impl Iterator<Item = u32> + '_ {
        self.counts.keys().copied()
    }

    /// Whether the hour read every owner anew.
    pub fn rebuilt(&self) -> bool {
        self.rebuilt
    }

    /// The states the hour gave another owner, added or removed, ascending.
    pub fn moved(&self) -> &[u32] {
        &self.moved
    }

    /// The canonical slots that began or stopped owning states this hour, ascending; every
    /// slot owning states before or after when [`Self::rebuilt`].
    pub fn reached(&self) -> &[u32] {
        &self.reached
    }

    /// Adds `delta` states to `slot`'s count, or to the unresolved.
    fn tally(&mut self, slot: Option<u32>, delta: i64) -> io::Result<()> {
        let apply = |count: u32| {
            u32::try_from(i64::from(count) + delta)
                .map_err(|_| invalid("A state owner's count left its range"))
        };
        let Some(slot) = slot else {
            self.unresolved = apply(self.unresolved)?;
            return Ok(());
        };
        let before = self.owned(slot);
        let after = apply(before)?;
        if after == 0 {
            self.counts.remove(&slot);
        } else {
            self.counts.insert(slot, after);
        }
        self.owning
            .try_reserve(1)
            .map_err(|_| engine_allocation::oom())?;
        self.owning.entry(slot).or_insert(before > 0);
        Ok(())
    }

    /// One more (`add`) or fewer state owned by `raw`.
    fn count(&mut self, raw: u32, add: bool, roster: &Roster) -> io::Result<()> {
        let slot = if add {
            let entry = self.raws.entry(raw).or_insert((0, roster.slot_of(raw)));
            entry.0 += 1;
            entry.1
        } else {
            let entry = self
                .raws
                .get_mut(&raw)
                .ok_or_else(|| invalid("A state's owner was never counted"))?;
            entry.0 -= 1;
            let slot = entry.1;
            if entry.0 == 0 {
                self.raws.remove(&raw);
            }
            slot
        };
        self.tally(slot, if add { 1 } else { -1 })
    }

    /// The owner of the state at States `slot`: its id and the owner's raw link id.
    fn read(&mut self, state: &State, slot: u32, format: FormatVersion) -> io::Result<(u32, u32)> {
        #[cfg(test)]
        {
            self.reads += 1;
        }
        let held = columns::table(state, TableId::States)?;
        let id = held
            .key(slot)
            .ok_or_else(|| invalid(format!("States slot {slot} holds no row")))?
            as u32;
        if !held.recorded(self.field, slot) {
            let row: StateRow = columns::row(state, TableId::States, slot, format.version())?;
            return Ok((id, row.owner));
        }
        let owner = *columns::plain::<u32>(held, self.field)?
            .get(slot as usize)
            .ok_or_else(|| invalid(format!("States slot {slot} holds no cell")))?;
        Ok((id, owner))
    }

    /// Every owner read anew; every slot owning states before or after reached.
    fn rebuild(
        &mut self,
        state: &State,
        complete: bool,
        roster: &Roster,
        format: FormatVersion,
    ) -> io::Result<()> {
        self.owning.clear();
        self.owning
            .try_reserve(self.counts.len())
            .map_err(|_| engine_allocation::oom())?;
        // Every slot owning before counts as reached, as every one owning after does.
        self.owning
            .extend(self.counts.keys().map(|&slot| (slot, false)));
        engine_allocation::reserve(&mut self.moved, self.owners.len())?;
        self.moved.extend(self.owners.keys().copied());
        self.owners.clear();
        self.raws.clear();
        self.counts.clear();
        self.unresolved = 0;
        self.complete = complete;
        self.rebuilt = true;
        if !complete {
            return Ok(());
        }
        let live: Vec<u32> = columns::table(state, TableId::States)?
            .live()
            .ones()
            .collect();
        self.owners
            .try_reserve(live.len())
            .map_err(|_| engine_allocation::oom())?;
        for slot in live {
            let (id, raw) = self.read(state, slot, format)?;
            self.owners.insert(id, raw);
            self.count(raw, true, roster)?;
            engine_allocation::reserve(&mut self.moved, 1)?;
            self.moved.push(id);
        }
        Ok(())
    }

    pub(crate) fn hour(
        &mut self,
        state: &State,
        facts: &Facts<'_>,
        roster: &Roster,
        format: FormatVersion,
    ) -> io::Result<()> {
        self.rebuilt = false;
        self.moved.clear();
        self.reached.clear();
        self.owning.clear();
        let complete = columns::complete(state, TableId::States);
        if facts.first() || complete != self.complete || roster.rebuilt() {
            self.rebuild(state, complete, roster, format)?;
        } else if complete {
            let mask = 1_u64 << self.field;
            if facts.touched(TableId::States, mask) {
                for &key in &columns::changes(state, TableId::States)?.deleted_keys {
                    if let Some(raw) = self.owners.remove(&(key as u32)) {
                        self.count(raw, false, roster)?;
                        engine_allocation::reserve(&mut self.moved, 1)?;
                        self.moved.push(key as u32);
                    }
                }
                for slot in edited(state, TableId::States, mask)? {
                    let (id, raw) = self.read(state, slot, format)?;
                    let old = self.owners.insert(id, raw);
                    if old == Some(raw) {
                        continue;
                    }
                    if let Some(old) = old {
                        self.count(old, false, roster)?;
                    }
                    self.count(raw, true, roster)?;
                    engine_allocation::reserve(&mut self.moved, 1)?;
                    self.moved.push(id);
                }
            }
            // A moved link moves every state its raw id owns.
            for &raw in roster.changed_links() {
                let Some(&(states, old)) = self.raws.get(&raw) else {
                    continue;
                };
                let now = roster.slot_of(raw);
                if now != old {
                    self.raws.insert(raw, (states, now));
                    self.tally(old, -i64::from(states))?;
                    self.tally(now, i64::from(states))?;
                }
            }
        }
        self.moved.sort_unstable();
        self.moved.dedup();
        engine_allocation::reserve(&mut self.reached, self.owning.len())?;
        let rebuilt = self.rebuilt;
        for (&slot, &before) in &self.owning {
            if rebuilt || (self.counts.get(&slot).copied().unwrap_or(0) > 0) != before {
                self.reached.push(slot);
            }
        }
        self.reached.sort_unstable();
        Ok(())
    }
}
