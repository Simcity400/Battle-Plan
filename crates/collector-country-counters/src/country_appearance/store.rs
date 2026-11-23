use super::Captured;
use crate::country_input_changes::{self as changes, Changes, Counts, Table};
use engine_allocation as allocation;
use engine_native_memory::{
    capture_text::Owned,
    country_identity::Country,
    cpdx_array::Array,
    walk::Outcome,
};
use engine_native::ccountry::{AppearanceGlobals, OwnedAppearanceRow};

#[derive(Default)]
struct HeldCountry {
    country: Option<(usize, u32)>,
    row: OwnedAppearanceRow,
    subjects: Vec<u32>,
    tag: Vec<u8>,
}

#[derive(Clone, Copy)]
struct Visit {
    country: Country,
    carried: bool,
}

#[derive(Default)]
pub struct AppearanceStore {
    epoch: Option<u64>,
    context: Option<(Array, u32)>,
    held: Vec<HeldCountry>,
    marked: Vec<u32>,
    visits: Vec<Visit>,
    untracked: bool,
    taken: Option<u64>,
    certified: bool,
    globals: Option<((Array, u32), u32)>,
    carrying: bool,
    bytes: usize,
}

pub struct Carry<'a> {
    pub store: &'a mut AppearanceStore,
    pub changes: &'a dyn Changes,
}

impl<'a> Carry<'a> {
    pub fn observed(store: &'a mut AppearanceStore) -> Self {
        Self {
            store,
            changes: &changes::Observer,
        }
    }
}

struct Fresh<'a> {
    row: &'a OwnedAppearanceRow,
    subjects: Option<&'a [u32]>,
    tag: Option<&'a [u8]>,
}

impl AppearanceStore {
    pub fn forget(&mut self) {
        self.epoch = None;
        self.context = None;
        for held in &mut self.held {
            held.country = None;
        }
    }

    pub fn allocated_bytes(&self) -> usize {
        self.bytes
    }

    fn measure(&self) -> usize {
        size_of::<Self>()
            + self.held.capacity() * size_of::<HeldCountry>()
            + self
                .held
                .iter()
                .map(|held| {
                    let row = &held.row;
                    row.cosmetic_tag.allocated_bytes()
                        + row.name_tag.allocated_bytes()
                        + row.leader_ideology.allocated_bytes()
                        + row.graphical_culture.allocated_bytes()
                        + row.graphical_culture_2d.allocated_bytes()
                        + row.old_names.as_ref().map_or(0, |names| {
                            names.iter().map(Owned::allocated_bytes).sum::<usize>()
                        })
                        + held.subjects.capacity() * size_of::<u32>()
                        + held.tag.capacity()
                })
                .sum::<usize>()
            + self.marked.capacity() * size_of::<u32>()
            + self.visits.capacity() * size_of::<Visit>()
    }

    pub(super) fn begin(&mut self, changes: &dyn Changes) {
        self.marked.clear();
        self.visits.clear();
        self.untracked = false;
        self.globals = None;
        self.carrying = false;
        let marked = &mut self.marked;
        let mut overflow = false;
        let epoch = changes.take(Table::Appearance, &mut |id| {
            let id = u32::try_from(id).ok().filter(|_| {
                allocation::advisory(|| allocation::reserve_push_batch(marked, 1)).is_ok()
            });
            match id {
                Some(id) => marked.push(id),
                None => overflow = true,
            }
        });
        marked.sort_unstable();
        marked.dedup();
        self.taken = if overflow { None } else { epoch };
        self.certified = self.taken.is_some() && self.taken == self.epoch;
    }

    pub(super) fn select(
        &mut self,
        globals: AppearanceGlobals,
        country: Country,
        row: &mut OwnedAppearanceRow,
    ) -> bool {
        if self.globals.is_none() {
            let context = (globals.naming.tags, globals.naming.dynamic_boundary);
            self.globals = Some((context, globals.links.count));
            self.carrying = self.certified && self.context == Some(context);
        }
        if self.untracked
            || allocation::advisory(|| allocation::reserve_push_batch(&mut self.visits, 1)).is_err()
        {
            self.untracked = true;
            return false;
        }
        let held = self
            .held
            .get(country.canonical_slot as usize)
            .filter(|held| held.country == Some((country.pointer, country.id)));
        let carried = self.carrying
            && self.marked.binary_search(&country.id).is_err()
            && held.is_some_and(|held| allocation::advisory(|| row.copy_held(&held.row)).is_ok());
        self.visits.push(Visit { country, carried });
        carried
    }

    fn carried(&self, index: usize) -> Option<&HeldCountry> {
        let visit = self.visits.get(index).filter(|visit| visit.carried)?;
        self.held.get(visit.country.canonical_slot as usize)
    }

    pub(super) fn carried_subjects(&self, index: usize) -> Option<&[u32]> {
        self.carried(index).map(|held| &held.subjects[..])
    }

    pub(super) fn carried_tag(&self, index: usize) -> Option<&[u8]> {
        self.carried(index).map(|held| &held.tag[..])
    }

    pub(super) fn finish(&mut self, captured: &Captured) {
        let walked = captured.rows.outcome.len().max(self.visits.len());
        let carried = self.visits.iter().filter(|visit| visit.carried).count();
        let counts = Counts {
            marked: self.marked.len() as u64,
            read: (walked - carried) as u64,
            full: !self.carrying,
            ..Counts::default()
        };
        let complete = !self.untracked
            && self.taken.is_some()
            && matches!(captured.rows.outcome, Outcome::Rows { len } if len == self.visits.len());
        let mut stored = complete;
        let mut touched = false;
        if complete {
            let arena = captured.naming.first.get(..captured.naming.len);
            let subjects = captured.subjects.first.get(..captured.subjects.len);
            let slots = self
                .visits
                .iter()
                .map(|visit| visit.country.canonical_slot as usize + 1)
                .max()
                .unwrap_or(0);
            if self.held.len() < slots {
                touched = true;
                if allocation::advisory(|| allocation::resize_default(&mut self.held, slots))
                    .is_err()
                {
                    stored = false;
                }
            }
            for (index, visit) in self.visits.iter().enumerate() {
                if visit.carried {
                    continue;
                }
                let row = &captured.rows.rows[index];
                let fresh = Fresh {
                    row,
                    subjects: captured.subject_starts.get(index).and_then(|&start| {
                        subjects?.get(start..start.checked_add(row.subjects.count as usize)?)
                    }),
                    tag: row
                        .original_tag
                        .and_then(|name| arena?.get(name.start..name.start.checked_add(name.len)?)),
                };
                let slot = visit.country.canonical_slot as usize;
                if let Some(held) = self.held.get_mut(slot) {
                    touched = true;
                    allocation::advisory(|| hold(held, visit.country, &fresh));
                }
            }
            let mut next = 0;
            for visit in &self.visits {
                let slot = visit.country.canonical_slot as usize;
                for held in self.held.get_mut(next..slot).into_iter().flatten() {
                    held.country = None;
                }
                next = slot + 1;
            }
            for held in self.held.get_mut(next..).into_iter().flatten() {
                held.country = None;
            }
        }
        if stored {
            self.epoch = self.taken;
            self.context = self.globals.map(|(context, _)| context);
        } else {
            self.forget();
        }
        let ids = complete.then(|| {
            let bound = self
                .visits
                .iter()
                .map(|visit| visit.country.id as usize + 1)
                .max()
                .unwrap_or(0);
            bound.max(self.globals.map_or(0, |(_, links)| links as usize))
        });
        changes::record(Table::Appearance, &counts, ids);
        if touched {
            self.bytes = self.measure();
        }
    }
}

fn hold(held: &mut HeldCountry, country: Country, fresh: &Fresh<'_>) -> bool {
    held.country = None;
    let (Some(subjects), Some(tag), Some(_)) = (fresh.subjects, fresh.tag, &fresh.row.old_names)
    else {
        return false;
    };
    let stored = (|| {
        held.row.copy_held(fresh.row).ok()?;
        held.subjects.clear();
        allocation::reserve(&mut held.subjects, subjects.len()).ok()?;
        held.subjects.extend_from_slice(subjects);
        held.tag.clear();
        allocation::reserve(&mut held.tag, tag.len()).ok()?;
        held.tag.extend_from_slice(tag);
        Some(())
    })();
    if stored.is_some() {
        held.country = Some((country.pointer, country.id));
    }
    stored.is_some()
}

#[cfg(test)]
mod tests;
