use collector_missions::mission_changes::{Changes, Counts};
use engine_allocation as allocation;
use engine_native_air::cair_regional_mission::{Carry, Registration, Root};
use std::ops::Range;
mod order;
pub use order::OrderLabel;

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

#[derive(Default)]
struct Held {
    entries: Vec<Entry>,
    registrations: Vec<Registration>,
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

    fn find(&self, certificate: &Certificate) -> Option<Range<usize>> {
        self.entry(certificate.root.address)
            .filter(|entry| entry.certificate == *certificate)
            .map(|entry| entry.registrations.clone())
    }

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

#[derive(Clone, Debug, Default)]
struct Plan {
    certificate: Option<Certificate>,
    held: Option<Range<usize>>,
    recorded: Option<usize>,
}

#[derive(Default)]
pub struct MissionStore {
    current: Held,
    previous: Held,
    plans: Vec<Plan>,
    counts: Counts,
    bytes: usize,
    forgotten: bool,
    orders: order::Orders,
    pub transfers: super::transfers::Store,
}

impl MissionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn forget(&mut self) {
        self.forgotten = true;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.bytes
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn counts(&self) -> Counts {
        self.counts
    }

    fn begin(&mut self, quiet: Option<(u64, u64)>) -> bool {
        if std::mem::take(&mut self.forgotten) {
            self.current.clear();
            self.previous.clear();
            self.orders.clear();
            self.transfers.clear();
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

    fn finish(&mut self, complete: bool) {
        let countries = self.plans.len();
        if complete {
            let entries = &mut self.current.entries;
            entries.sort_unstable_by_key(|entry| entry.certificate.root.address);
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
        self.measure_transfers();
        // SAFETY: Called from admitted air-mission capture with measured country demand.
        unsafe {
            collector_missions::mission_changes::record(&self.counts, complete.then_some(countries));
        }
    }

    pub fn measure_transfers(&mut self) {
        self.bytes = self.current.measure()
            + self.previous.measure()
            + self.plans.capacity() * size_of::<Plan>()
            + self.orders.allocated_bytes()
            + self.transfers.allocated_bytes();
    }
}

pub struct MissionReuse<'a> {
    store: &'a mut MissionStore,
    changes: &'a dyn Changes,
    quiet: bool,
}

impl<'a> MissionReuse<'a> {
    /// # Safety
    /// Caller holds hourly capture admission for the active session (or isolated fixture control)
    /// while mission watch storage remains live.
    pub unsafe fn begin(store: &'a mut MissionStore, changes: &'a dyn Changes) -> Self {
        let quiet = store.begin(changes.quiet());
        Self {
            store,
            changes,
            quiet,
        }
    }

    /// # Safety
    /// Caller holds hourly capture admission for the active session (or isolated fixture control)
    /// so recording global `mission_changes` sizing demand does not corrupt a live session.
    #[cfg(any(test, feature = "test-support"))]
    pub unsafe fn finish(self, complete: bool) {
        self.store.finish(complete);
    }

    /// # Safety
    /// Caller holds hourly capture admission for the active session (or isolated fixture control)
    /// so recording global `mission_changes` sizing demand does not corrupt a live session.
    pub unsafe fn finish_with_order(
        self,
        complete: bool,
        memory: &impl engine_native_memory::Memory,
        wings: &[engine_native_air::cair_regional_mission::WingIdentity],
        output: &mut Vec<OrderLabel>,
    ) {
        self.store.finish(complete);
        let store = self.store;
        output.clear();
        if !complete {
            store.orders.clear();
        } else {
            store.orders.capture(
                memory,
                wings,
                &store.current.entries,
                &store.current.registrations,
                output,
            );
        }
        store.measure_transfers();
    }

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
    fn plan(&mut self, root: &Root) -> (usize, bool) {
        let certificate = self.certify(root);
        let held = certificate.and_then(|certificate| self.store.previous.find(&certificate));
        let carry = held.is_some();
        let token = self.store.plans.len();
        let planned = allocation::advisory(|| allocation::reserve(&mut self.store.plans, 1));
        if planned.is_err() {
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
        let current = &mut store.current;
        let certificate = plan.certificate.filter(|_| {
            allocation::advisory(|| allocation::reserve(&mut current.entries, 1)).is_ok()
        });
        match certificate {
            Some(certificate) => current.entries.push(Entry {
                certificate,
                registrations: fresh,
            }),
            None => current.registrations.truncate(start),
        }
    }
}
