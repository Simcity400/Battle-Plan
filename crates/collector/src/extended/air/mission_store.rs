//! AirRegionalMissions registrations held between hours per country air
//! object ([`crate::mission_changes`]).
//!
//! The walk still visits every country air object and gathers its regional, all-active and
//! active-region headers (the strategic air's mission vector headers) each hour. An object's held
//! registrations (region id, vector ordinal and mission pointer) are carried, without reading its
//! active-region ids, regional vectors or mission pointers, only when all of these hold at this
//! boundary: the same object address, canonical slot and gathered header bytes as the held read,
//! and no hooked writer touched the object since that read registered it (same watch epoch and
//! sequence; a load, gap, recovery, table change, poison or running writer revokes it). register
//! mission and unregister mission, both hooked, are the only run-time writers of the regional and
//! all-active vectors; the carrier mission pass also inserts active-region ids, which moves that
//! header ([`engine::regional_mission_mutations`]), so equal headers list the same ids. The walk
//! still checks each carried registration's wing in this hour's census exactly as it checks a read
//! one and reads the object afresh on any disagreement
//! ([`engine::cair_regional_mission::walk_carried`]), so the published rows and outcome are a fresh
//! walk's.
//!
//! When the domain's quiet token is the one taken before the held walk's
//! first registration, no hooked writer ran since: an object with the held
//! address, slot and headers keeps its held certificate without a lookup.
//!
//! Each hour also reads the carryable objects of its sampled slice
//! ([`crate::sampled_verify`], keyed by canonical slot; every one under
//! `mission_verify`) afresh, publishes the fresh rows and poisons reuse on a
//! difference. The store travels with the AirMissions job's request and
//! holds only the objects of the latest complete walk, so its size follows
//! live demand. Allocation is advisory: an object whose registrations cannot
//! be held is read afresh next hour.
use crate::{
    mission_changes::{Changes, Counts},
    sampled_verify::Check,
};
use engine::{
    allocation,
    cair_regional_mission::{Carry, Registration, Root},
};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Certificate {
    epoch: u64,
    sequence: u64,
    root: Root,
}

#[derive(Clone, Debug)]
struct Entry {
    certificate: Certificate,
    registrations: Range<usize>,
}

/// One walk's held registrations.
#[derive(Default)]
struct Held {
    /// Sorted by object address once the walk finishes.
    entries: Vec<Entry>,
    registrations: Vec<Registration>,
    /// The domain's quiet token before the walk's first registration.
    quiet: Option<(u64, u64)>,
}

impl Held {
    fn clear(&mut self) {
        self.entries.clear();
        self.registrations.clear();
        self.quiet = None;
    }

    fn entry(&self, address: usize) -> Option<&Entry> {
        let index = self
            .entries
            .binary_search_by_key(&address, |entry| entry.certificate.root.address)
            .ok()?;
        Some(&self.entries[index])
    }

    /// The held registrations of exactly `certificate`.
    fn find(&self, certificate: &Certificate) -> Option<Range<usize>> {
        self.entry(certificate.root.address)
            .filter(|entry| entry.certificate == *certificate)
            .map(|entry| entry.registrations.clone())
    }

    /// Holds `registrations` for `certificate`; `false` when the storage
    /// could not grow, which only withholds the certificate.
    fn hold(&mut self, certificate: Certificate, registrations: &[Registration]) -> bool {
        let stored = allocation::advisory(|| {
            allocation::reserve(&mut self.entries, 1)?;
            allocation::reserve(&mut self.registrations, registrations.len())?;
            std::io::Result::Ok(())
        });
        if stored.is_err() {
            return false;
        }
        let first = self.registrations.len();
        self.registrations.extend_from_slice(registrations);
        self.entries.push(Entry {
            certificate,
            registrations: first..self.registrations.len(),
        });
        true
    }

    fn measure(&self) -> usize {
        self.entries.capacity() * size_of::<Entry>()
            + self.registrations.capacity() * size_of::<Registration>()
    }
}

/// One planned object: its certificate, carryable held registrations and,
/// when read afresh, where its recorded registrations start in the walk
/// being made (`None` once one could not be recorded).
#[derive(Clone, Debug, Default)]
struct Plan {
    certificate: Option<Certificate>,
    held: Option<Range<usize>>,
    recorded: Option<usize>,
}

/// Registrations of the latest complete walk and the walk being made.
#[derive(Default)]
pub(crate) struct MissionStore {
    current: Held,
    previous: Held,
    /// This walk's plans, indexed by token.
    plans: Vec<Plan>,
    counts: Counts,
    /// Allocated bytes as of the last finish, for the game thread.
    bytes: usize,
    /// Set on the game thread; the next walk drops every held registration.
    forgotten: bool,
}

impl MissionStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Nothing held certifies anything: the next walk reads afresh.
    /// Allocation-free for the game thread.
    pub(crate) fn forget(&mut self) {
        self.forgotten = true;
    }

    /// As of the last finish, so the game thread need not measure.
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.bytes
    }

    #[cfg(test)]
    pub(crate) fn counts(&self) -> Counts {
        self.counts
    }

    /// `quiet` is the domain's token before this walk's first registration.
    /// Returns whether it proves every held certificate current.
    fn begin(&mut self, quiet: Option<(u64, u64)>) -> bool {
        if std::mem::take(&mut self.forgotten) {
            self.current.clear();
            self.previous.clear();
        }
        std::mem::swap(&mut self.current, &mut self.previous);
        self.current.clear();
        self.current.quiet = quiet;
        self.plans.clear();
        self.counts = Counts::default();
        let unchanged = quiet.is_some() && quiet == self.previous.quiet;
        self.counts.quiet = unchanged;
        unchanged
    }

    /// Keeps this walk's registrations only when it completed, and publishes
    /// its counts.
    fn finish(&mut self, complete: bool) {
        let countries = self.plans.len();
        if complete {
            let entries = &mut self.current.entries;
            entries.sort_unstable_by_key(|entry| entry.certificate.root.address);
            // An object occurring twice names no single held read.
            let mut index = 0;
            while index < entries.len() {
                let address = entries[index].certificate.root.address;
                let end = entries[index..]
                    .iter()
                    .position(|entry| entry.certificate.root.address != address)
                    .map_or(entries.len(), |offset| index + offset);
                if end - index > 1 {
                    entries.drain(index..end);
                } else {
                    index = end;
                }
            }
        } else {
            self.current.clear();
        }
        self.previous.clear();
        self.plans.clear();
        self.bytes = self.current.measure()
            + self.previous.measure()
            + self.plans.capacity() * size_of::<Plan>();
        crate::mission_changes::record(&self.counts, complete.then_some(countries));
    }
}

/// A country air object's stable sampling key: its canonical slot.
pub(crate) fn key(root: &Root) -> u64 {
    u64::from(root.canonical_slot)
}

/// One walk's view of the store and the observation that certifies reuse.
pub(crate) struct MissionReuse<'a> {
    store: &'a mut MissionStore,
    changes: &'a dyn Changes,
    /// Which carryable objects this hour reads afresh and compares.
    check: Check,
    /// No hooked writer ran since the held walk began.
    quiet: bool,
}

impl<'a> MissionReuse<'a> {
    /// Takes the domain's quiet token before the walk's first registration.
    pub(crate) fn begin(
        store: &'a mut MissionStore,
        changes: &'a dyn Changes,
        check: Check,
    ) -> Self {
        let quiet = store.begin(changes.quiet());
        Self {
            store,
            changes,
            check,
            quiet,
        }
    }

    /// Ends the walk: `complete` when every object was read or carried whole.
    pub(crate) fn finish(self, complete: bool) {
        self.store.finish(complete);
    }

    /// This hour's certificate for `root`: the held one while the domain is
    /// quiet, otherwise its registration.
    fn certify(&self, root: &Root) -> Option<Certificate> {
        if self.quiet
            && let Some(entry) = self.store.previous.entry(root.address)
            && entry.certificate.root == *root
        {
            return Some(entry.certificate);
        }
        self.changes
            .sequence(root.address)
            .map(|(epoch, sequence)| Certificate {
                epoch,
                sequence,
                root: *root,
            })
    }
}

impl Carry for MissionReuse<'_> {
    /// Registration precedes every read it certifies.
    fn plan(&mut self, root: &Root) -> (usize, bool) {
        let certificate = self.certify(root);
        let held = certificate.and_then(|certificate| self.store.previous.find(&certificate));
        let carry = held.is_some() && !self.check.covers(key(root));
        let token = self.store.plans.len();
        let planned = allocation::advisory(|| allocation::reserve(&mut self.store.plans, 1));
        if planned.is_err() {
            // Unplanned: read afresh and held for nothing.
            return (usize::MAX, false);
        }
        let recorded = Some(self.store.current.registrations.len());
        self.store.plans.push(Plan {
            certificate,
            held,
            recorded,
        });
        (token, carry)
    }

    fn held(&self, token: usize) -> &[Registration] {
        match self
            .store
            .plans
            .get(token)
            .and_then(|plan| plan.held.clone())
        {
            Some(range) => &self.store.previous.registrations[range],
            None => &[],
        }
    }

    fn carried(&mut self, token: usize) {
        let MissionStore {
            previous,
            current,
            plans,
            counts,
            ..
        } = &mut *self.store;
        let Some(Plan {
            certificate: Some(certificate),
            held: Some(range),
            ..
        }) = plans.get(token).cloned()
        else {
            return;
        };
        let registrations = &previous.registrations[range];
        counts.reused += 1;
        counts.reused_rows += registrations.len() as u64;
        // A failed hold only withholds the next hour's certificate.
        let _ = current.hold(certificate, registrations);
    }

    fn record(&mut self, token: usize, registration: Registration) {
        let store = &mut *self.store;
        store.counts.read_rows += 1;
        let Some(plan) = store.plans.get_mut(token) else {
            return;
        };
        let Some(start) = plan.recorded else {
            return;
        };
        let registrations = &mut store.current.registrations;
        if allocation::advisory(|| allocation::reserve(registrations, 1)).is_err() {
            // Neither compared nor held.
            registrations.truncate(start);
            plan.recorded = None;
            return;
        }
        registrations.push(registration);
    }

    fn read(&mut self, token: usize) {
        let store = &mut *self.store;
        store.counts.read += 1;
        let Some(plan) = store.plans.get(token).cloned() else {
            return;
        };
        let Some(start) = plan.recorded else {
            return;
        };
        let fresh = start..store.current.registrations.len();
        if let Some(held) = plan.held {
            store.counts.verified += 1;
            if store.previous.registrations[held] != store.current.registrations[fresh.clone()] {
                store.counts.mismatches += 1;
                self.changes.mismatch();
                // Poisoned: no later object is certified, quiet or not.
                self.quiet = false;
            }
        }
        let current = &mut store.current;
        let certificate = plan.certificate.filter(|_| {
            allocation::advisory(|| allocation::reserve(&mut current.entries, 1)).is_ok()
        });
        match certificate {
            Some(certificate) => current.entries.push(Entry {
                certificate,
                registrations: fresh,
            }),
            // Held for nothing: drop what it recorded.
            None => current.registrations.truncate(start),
        }
    }
}
