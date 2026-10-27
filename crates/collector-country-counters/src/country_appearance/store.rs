//! CountryAppearance rows held between hours
//! ([`crate::country_input_changes::Table::Appearance`]).
//!
//! The Countries job walks the country census every hour. A country's
//! appearance (colours, cosmetic tag, name operands, graphical cultures, the
//! ruling party's first leader, overlord, retained old names, original tag and
//! subject roster) is read afresh only when a hooked writer marked it since
//! the previous take, it is new to the census or its last read was not held.
//! Every other country's row, subject payload and original tag bytes are
//! copied from its held read into the hour's arenas at the place a full read
//! puts them, so the published table equals a full read of the same memory.
//! A country is carried only while the hour's take returned the epoch its
//! held read was certified under, the walk's naming globals (tag string
//! header and dynamic tag boundary) are the held ones, and the census names
//! the held object and native id at its canonical slot.
//!
//! Only a complete read under a take that returned an epoch is held: a
//! failed or incomplete one, or a failed job, holds nothing, so the next hour
//! reads every country. A row that cannot be copied is read instead. Storage
//! is advisory and worker-only.
use super::Captured;
use crate::country_input_changes::{self as changes, Changes, Counts, Table};
use engine_allocation as allocation;
use engine_native::{
    capture_text::Owned, ccountry::AppearanceGlobals, ccountry::Country,
    ccountry::OwnedAppearanceRow, cpdx_array::Array, walk::Outcome,
};

/// One canonical slot's held read.
#[derive(Default)]
struct HeldCountry {
    /// The census object and native id read; `None` holds nothing.
    country: Option<(usize, u32)>,
    row: OwnedAppearanceRow,
    /// The subject roster payload, in native order.
    subjects: Vec<u32>,
    /// The original tag's bytes; the row's span names the read's arena.
    tag: Vec<u8>,
}

/// What the hour did with one walked country, in walk order.
#[derive(Clone, Copy)]
struct Visit {
    country: Country,
    /// Copied from its held read; nothing of it was read.
    carried: bool,
}

/// The latest held read per canonical slot, and the hour's selection.
#[derive(Default)]
pub struct AppearanceStore {
    /// The take epoch and naming globals the held reads were certified under.
    epoch: Option<u64>,
    context: Option<(Array, u32)>,
    /// Indexed by canonical slot.
    held: Vec<HeldCountry>,
    /// The hour's marked native ids, ascending.
    marked: Vec<u32>,
    /// The hour's walked countries, in walk order (the rows' order).
    visits: Vec<Visit>,
    /// A visit could not be recorded: nothing later is carried or held.
    untracked: bool,
    /// The hour's take epoch, `None` when it certified nothing.
    taken: Option<u64>,
    /// The take returned the held epoch.
    certified: bool,
    /// The walk's naming globals and link count, once seen.
    globals: Option<((Array, u32), u32)>,
    /// Held reads may be carried this hour.
    carrying: bool,
    /// [`Self::allocated_bytes`], measured on the worker after each hour.
    bytes: usize,
}

/// A store on loan to one hour's appearance read, with the domain it takes
/// its marks from.
pub struct Carry<'a> {
    pub store: &'a mut AppearanceStore,
    pub changes: &'a dyn Changes,
}

impl<'a> Carry<'a> {
    /// The hooks' domain.
    pub fn observed(store: &'a mut AppearanceStore) -> Self {
        Self {
            store,
            changes: &changes::Observer,
        }
    }
}

/// One freshly read row with its payload and original tag bytes.
struct Fresh<'a> {
    row: &'a OwnedAppearanceRow,
    subjects: Option<&'a [u32]>,
    tag: Option<&'a [u8]>,
}

impl AppearanceStore {
    /// Nothing held certifies anything: the next hour reads every country.
    pub fn forget(&mut self) {
        self.epoch = None;
        self.context = None;
        for held in &mut self.held {
            held.country = None;
        }
    }

    /// The storage measured after the latest hour, for the game thread's
    /// accounting.
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

    /// The hour's take of the marked countries, before the walk.
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

    /// The walk offers `country` with its row's storage: fills the row from
    /// the held read and returns true when the country is carried.
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

    /// The held read of row `index` (walk order), when that row was carried.
    fn carried(&self, index: usize) -> Option<&HeldCountry> {
        let visit = self.visits.get(index).filter(|visit| visit.carried)?;
        self.held.get(visit.country.canonical_slot as usize)
    }

    /// The held subject payload of row `index` when it was carried.
    pub(super) fn carried_subjects(&self, index: usize) -> Option<&[u32]> {
        self.carried(index).map(|held| &held.subjects[..])
    }

    /// The held original tag bytes of row `index` when it was carried.
    pub(super) fn carried_tag(&self, index: usize) -> Option<&[u8]> {
        self.carried(index).map(|held| &held.tag[..])
    }

    /// After the hour's appearance read: holds every complete fresh read and
    /// records the hour.
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
        // Storage moves only when a read is held.
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
            // A slot the census no longer names holds nothing.
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
            // Every native id indexes the link table, dynamic countries included.
            bound.max(self.globals.map_or(0, |(_, links)| links as usize))
        });
        changes::record(Table::Appearance, &counts, ids);
        if touched {
            self.bytes = self.measure();
        }
    }
}

/// Holds a complete fresh read of `country`; false when it is incomplete or
/// cannot be stored, which leaves the slot holding nothing.
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
