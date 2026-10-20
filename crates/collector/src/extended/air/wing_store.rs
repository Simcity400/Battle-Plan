//! WingEquipment rows held between hours per air wing
//! ([`crate::wing_changes`]).
//!
//! The air pass still walks every wing each hour (AirWings, WingCumulative, theatre memberships and
//! game-score inputs need it). A wing's held rows are carried, without reading its equipment pool's
//! vtable, header, entries or their variants, only when all of these hold at this boundary: the
//! same wing address and id as the held read, and no hooked writer touched its pool
//! (`cair_wing::EQUIPMENT_POOL`) since that read registered it (same watch epoch and sequence; a
//! load, gap, recovery, table change, poison or running writer revokes it). Every writer of a
//! wing's pool is a hooked site ([`engine_mutations::air_wing_equipment_mutations`]), and a wing built at a
//! destroyed wing's address carries that destructor's notification, so a carried wing's rows equal
//! a fresh read's. Each hour also reads the carryable wings of its sampled slice
//! ([`crate::sampled_verify`], keyed by wing id; every one under `wing_verify`) afresh, publishes
//! the fresh rows and poisons reuse on a difference.
//!
//! The store travels with the AirWing job's payload and holds only the wings
//! of the latest complete pass, so its size follows live demand. Allocation
//! is advisory: a wing whose rows cannot be held is read afresh next hour.
//! Each part of a census split by owner slots has its own store; the census
//! keeps every part's rows only when it was complete as a whole
//! ([`WingStore::conclude`]). A wing whose owner moved to another part is
//! read afresh by that part's store once.
use crate::{
    sampled_verify::Check,
    wing_changes::{Changes, Counts},
};
use engine_allocation as allocation;
use engine_mutations::air_wing_equipment_mutations::POOL;
use engine_native::walk::Outcome;
use replay_data::rows::WingEquipmentRow;
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Certificate {
    epoch: u64,
    sequence: u64,
    wing: usize,
    id: u64,
}

#[derive(Clone, Debug)]
struct Entry {
    certificate: Certificate,
    rows: Range<usize>,
    /// During a pass only: range into the changed-read scratch, not held rows.
    fresh: bool,
}

/// One pass's held rows.
#[derive(Default, Debug)]
struct Held {
    /// Sorted by wing once the pass finishes.
    entries: Vec<Entry>,
    rows: Vec<WingEquipmentRow>,
}

impl Held {
    fn clear(&mut self) {
        self.entries.clear();
        self.rows.clear();
    }

    /// The held rows of exactly `certificate`.
    fn find(&self, certificate: &Certificate) -> Option<Range<usize>> {
        let index = self
            .entries
            .binary_search_by_key(&certificate.wing, |entry| entry.certificate.wing)
            .ok()?;
        let entry = &self.entries[index];
        (entry.certificate == *certificate).then(|| entry.rows.clone())
    }

    fn measure(&self) -> usize {
        self.entries.capacity() * size_of::<Entry>()
            + self.rows.capacity() * size_of::<WingEquipmentRow>()
    }
}

/// Rows of the latest complete pass and the pass being made.
#[derive(Default, Debug)]
pub(crate) struct WingStore {
    current: Held,
    previous: Held,
    counts: Counts,
    wings: usize,
    /// Allocated bytes as of the last settlement, for the game thread.
    bytes: usize,
    /// Set on the game thread; the next pass drops every held row.
    forgotten: bool,
    /// Only changed reads need row scratch; carried and equal verified rows
    /// remain ranges into the preceding whole buffer until settlement.
    fresh: Vec<WingEquipmentRow>,
}

impl WingStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Nothing held certifies anything: the next pass reads afresh.
    /// Allocation-free for the game thread.
    pub(crate) fn forget(&mut self) {
        self.forgotten = true;
    }

    /// As of the last settlement, so the game thread need not measure.
    pub(crate) fn allocated_bytes(&self) -> usize {
        self.bytes
    }

    #[cfg(test)]
    pub(crate) fn counts(&self) -> Counts {
        self.counts
    }

    #[cfg(test)]
    pub(super) fn held_rows(&self) -> &[WingEquipmentRow] {
        &self.current.rows
    }

    fn begin(&mut self) {
        if std::mem::take(&mut self.forgotten) {
            self.current.clear();
            self.previous.clear();
        }
        std::mem::swap(&mut self.current, &mut self.previous);
        self.current.clear();
        self.counts = Counts::default();
        self.wings = 0;
        self.fresh.clear();
    }

    /// Indexes this pass's rows for the next hour; [`Self::conclude`]
    /// decides whether they are kept.
    fn settle(&mut self) {
        let entries = &mut self.current.entries;
        entries.sort_unstable_by_key(|entry| entry.certificate.wing);
        // A wing occurring twice names no single held read.
        let mut index = 0;
        while index < entries.len() {
            let wing = entries[index].certificate.wing;
            let end = entries[index..]
                .iter()
                .position(|entry| entry.certificate.wing != wing)
                .map_or(entries.len(), |offset| index + offset);
            if end - index > 1 {
                entries.drain(index..end);
            } else {
                index = end;
            }
        }
        let live_rows = self
            .current
            .entries
            .iter()
            .map(|entry| entry.rows.len())
            .sum();
        let carried = self.current.entries.iter().all(|entry| !entry.fresh);
        let fresh = self.current.entries.iter().all(|entry| entry.fresh);
        if carried && live_rows == self.previous.rows.len() {
            std::mem::swap(&mut self.current.rows, &mut self.previous.rows);
        } else if fresh && live_rows == self.fresh.len() {
            std::mem::swap(&mut self.current.rows, &mut self.fresh);
        } else if allocation::advisory(|| allocation::reserve(&mut self.current.rows, live_rows))
            .is_ok()
        {
            // Only a changed pass or withdrawal merges ranges. Each live row
            // is copied once, whether it came from a fresh or a held read.
            for entry in &mut self.current.entries {
                let source = if entry.fresh {
                    &self.fresh
                } else {
                    &self.previous.rows
                };
                let start = self.current.rows.len();
                self.current
                    .rows
                    .extend_from_slice(&source[entry.rows.clone()]);
                entry.rows = start..self.current.rows.len();
            }
        } else {
            self.current.clear();
        }
        if !carried {
            for entry in &mut self.current.entries {
                entry.fresh = false;
            }
        }
        self.previous.clear();
        self.fresh.clear();
        self.measure();
    }

    /// A carried range needs only an entry, not another copy of its rows.
    fn hold_carried(&mut self, certificate: Certificate, rows: Range<usize>) {
        if allocation::advisory(|| allocation::reserve(&mut self.current.entries, 1)).is_ok() {
            self.current.entries.push(Entry {
                certificate,
                rows,
                fresh: false,
            });
        }
    }

    fn hold_read(&mut self, certificate: Certificate, rows: &[WingEquipmentRow]) {
        let stored = allocation::advisory(|| {
            allocation::reserve(&mut self.current.entries, 1)?;
            allocation::reserve(&mut self.fresh, rows.len())
        });
        if stored.is_ok() {
            let start = self.fresh.len();
            self.fresh.extend_from_slice(rows);
            self.current.entries.push(Entry {
                certificate,
                rows: start..self.fresh.len(),
                fresh: true,
            });
        }
    }

    fn measure(&mut self) {
        self.bytes = self.current.measure()
            + self.previous.measure()
            + self.fresh.capacity() * size_of::<WingEquipmentRow>();
    }

    /// This pass's counts and the wings it planned.
    pub(crate) fn tally(&self) -> (Counts, usize) {
        (self.counts, self.wings)
    }

    /// Keeps the settled pass's rows only when every wing of the census, in
    /// every part, was read or carried completely. Allocation-free for the
    /// game thread, which concludes a split census.
    pub(crate) fn conclude(&mut self, complete: bool) {
        if !complete {
            self.current.clear();
        }
        self.measure();
    }
}

/// One wing's plan: carry its held rows, or read it afresh.
#[derive(Clone, Debug)]
pub(crate) struct Plan {
    certificate: Option<Certificate>,
    /// The carryable held rows, indexing the previous pass's rows.
    held: Option<Range<usize>>,
    /// Whether the held rows may be carried without a fresh read.
    carry: bool,
}

impl Plan {
    /// Rows the caller must have room for to carry the wing, when it may.
    pub(crate) fn carry_rows(&self) -> Option<usize> {
        self.held.as_ref().filter(|_| self.carry).map(Range::len)
    }
}

/// One pass's view of the store and the observation that certifies reuse.
pub(crate) struct WingReuse<'a> {
    store: &'a mut WingStore,
    changes: &'a dyn Changes,
    /// Which carryable wings this hour reads afresh and compares.
    check: Check,
}

impl<'a> WingReuse<'a> {
    pub(crate) fn begin(store: &'a mut WingStore, changes: &'a dyn Changes, check: Check) -> Self {
        store.begin();
        Self {
            store,
            changes,
            check,
        }
    }

    /// Registers one wing's pool and plans its rows: carryable when the held
    /// read of the same wing and id is proved current, and carried without a
    /// read unless the hour's [`Check`] covers it. Registration precedes
    /// every read it certifies.
    pub(crate) fn plan(&mut self, wing: usize, id: u64) -> Plan {
        self.store.wings += 1;
        let certificate =
            self.changes
                .sequence(wing.wrapping_add(POOL))
                .map(|(epoch, sequence)| Certificate {
                    epoch,
                    sequence,
                    wing,
                    id,
                });
        let held = certificate.and_then(|certificate| self.store.previous.find(&certificate));
        Plan {
            certificate,
            carry: held.is_some() && !self.check.covers(id),
            held,
        }
    }

    /// The carried rows of a plan whose [`Plan::carry_rows`] the caller has
    /// room for, held again for the next hour.
    pub(crate) fn carry(&mut self, plan: &Plan) -> &[WingEquipmentRow] {
        let (Some(certificate), Some(range)) = (plan.certificate, plan.held.clone()) else {
            return &[];
        };
        self.store.counts.reused += 1;
        self.store.counts.reused_rows += range.len() as u64;
        self.store.hold_carried(certificate, range.clone());
        &self.store.previous.rows[range]
    }

    /// A fresh read of a planned wing: `rows` are its rows in pool order when
    /// `outcome` is complete. They are compared with carryable held rows (a
    /// difference poisons reuse) and held for the next hour.
    pub(crate) fn read(&mut self, plan: &Plan, outcome: Outcome, rows: &[WingEquipmentRow]) {
        let store = &mut *self.store;
        store.counts.read += 1;
        store.counts.read_rows += rows.len() as u64;
        if let Some(range) = plan.held.clone() {
            store.counts.verified += 1;
            if matches!(outcome, Outcome::Rows { .. })
                && store.previous.rows[range.clone()] == *rows
            {
                if let Some(certificate) = plan.certificate {
                    store.hold_carried(certificate, range);
                }
                return;
            }
            store.counts.mismatches += 1;
            self.changes.mismatch();
        }
        if let (Some(certificate), Outcome::Rows { .. }) = (plan.certificate, outcome) {
            store.hold_read(certificate, rows);
        }
    }

    /// Ends this part's pass; its store keeps the rows until the census
    /// concludes ([`WingStore::conclude`]).
    pub(crate) fn settle(self) {
        self.store.settle();
    }
}
