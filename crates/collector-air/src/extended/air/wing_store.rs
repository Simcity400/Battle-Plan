use collector_observers::wing_changes::{Changes, Counts};
use engine_allocation as allocation;
use engine_mutations::air_wing_equipment_mutations::POOL;
use engine_native_memory::walk::Outcome;
use replay_data::rows::WingEquipmentRow;
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Certificate {
    epoch: u64,
    sequence: u64,
    wing: usize,
    id: u64,
}

#[derive(Clone, Debug)]
struct Entry {
    certificate: Certificate,
    slot: usize,
    rows: Range<usize>,
    fresh: bool,
}

#[derive(Default, Debug)]
struct Held {
    entries: Vec<Entry>,
    rows: Vec<WingEquipmentRow>,
}

impl Held {
    fn clear(&mut self) {
        self.entries.clear();
        self.rows.clear();
    }

    fn find_by_wing(&self, wing: usize, cursor: &mut usize) -> Option<&Entry> {
        if let Some(entry) = self.entries.get(*cursor)
            && entry.certificate.wing == wing
        {
            *cursor += 1;
            return Some(entry);
        }
        let index = self
            .entries
            .binary_search_by_key(&wing, |entry| entry.certificate.wing)
            .ok()?;
        *cursor = index + 1;
        Some(&self.entries[index])
    }

    fn measure(&self) -> usize {
        self.entries.capacity() * size_of::<Entry>()
            + self.rows.capacity() * size_of::<WingEquipmentRow>()
    }
}

#[derive(Default, Debug)]
pub struct WingStore {
    current: Held,
    previous: Held,
    cursor: usize,
    counts: Counts,
    wings: usize,
    bytes: usize,
    forgotten: bool,
    fresh: Vec<WingEquipmentRow>,
}

impl WingStore {
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
        self.cursor = 0;
        self.counts = Counts::default();
        self.wings = 0;
        self.fresh.clear();
    }

    fn settle(&mut self) {
        let entries = &mut self.current.entries;
        entries.sort_unstable_by_key(|entry| entry.certificate.wing);
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

    fn hold_carried(&mut self, certificate: Certificate, slot: usize, rows: Range<usize>) {
        if allocation::advisory(|| allocation::reserve(&mut self.current.entries, 1)).is_ok() {
            self.current.entries.push(Entry {
                certificate,
                slot,
                rows,
                fresh: false,
            });
        }
    }

    fn hold_read(&mut self, certificate: Certificate, slot: usize, rows: &[WingEquipmentRow]) {
        let stored = allocation::advisory(|| {
            allocation::reserve(&mut self.current.entries, 1)?;
            allocation::reserve(&mut self.fresh, rows.len())
        });
        if stored.is_ok() {
            let start = self.fresh.len();
            self.fresh.extend_from_slice(rows);
            self.current.entries.push(Entry {
                certificate,
                slot,
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

    pub fn tally(&self) -> (Counts, usize) {
        (self.counts, self.wings)
    }

    pub fn conclude(&mut self, complete: bool) {
        if !complete {
            self.current.clear();
        }
        self.measure();
    }
}

#[derive(Clone, Debug)]
pub struct Plan {
    certificate: Option<Certificate>,
    slot: usize,
    held: Option<Range<usize>>,
}

impl Plan {
    pub fn carry_rows(&self) -> Option<usize> {
        self.held.as_ref().map(Range::len)
    }
}

pub struct WingReuse<'a> {
    store: &'a mut WingStore,
    changes: &'a dyn Changes,
}

impl<'a> WingReuse<'a> {
    pub fn begin(store: &'a mut WingStore, changes: &'a dyn Changes) -> Self {
        store.begin();
        Self { store, changes }
    }

    pub fn plan(&mut self, wing: usize, id: u64) -> Plan {
        self.store.wings += 1;
        let found = self
            .store
            .previous
            .find_by_wing(wing, &mut self.store.cursor);
        let held_slot = found.map_or(usize::MAX, |entry| entry.slot);
        let (certificate, slot) = match self
            .changes
            .sequence_at(wing.wrapping_add(POOL), held_slot)
        {
            Some(((epoch, sequence), slot)) => (
                Some(Certificate {
                    epoch,
                    sequence,
                    wing,
                    id,
                }),
                slot,
            ),
            None => (None, usize::MAX),
        };
        let held = certificate.and_then(|certificate| {
            found
                .filter(|entry| entry.certificate == certificate)
                .map(|entry| entry.rows.clone())
        });
        Plan {
            certificate,
            slot,
            held,
        }
    }

    pub fn carry(&mut self, plan: &Plan) -> &[WingEquipmentRow] {
        let (Some(certificate), Some(range)) = (plan.certificate, plan.held.clone()) else {
            return &[];
        };
        self.store.counts.reused += 1;
        self.store.counts.reused_rows += range.len() as u64;
        self.store.hold_carried(certificate, plan.slot, range.clone());
        &self.store.previous.rows[range]
    }

    pub fn read(&mut self, plan: &Plan, outcome: Outcome, rows: &[WingEquipmentRow]) {
        let store = &mut *self.store;
        store.counts.read += 1;
        store.counts.read_rows += rows.len() as u64;
        if let (Some(certificate), Outcome::Rows { .. }) = (plan.certificate, outcome) {
            store.hold_read(certificate, plan.slot, rows);
        }
    }

    pub fn settle(self) {
        self.store.settle();
    }
}
