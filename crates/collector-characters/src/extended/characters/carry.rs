use super::*;
use collector_observers::character_changes::{Changes, Counts, Table};
use std::ops::Range;

const UNHELD: usize = usize::MAX;

#[derive(Clone, Debug)]
struct Entry {
    pointer: usize,
    id: u32,
    certificate: (u64, u64),
    row: ccharacter::Row,
    bytes: Range<usize>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct Visit {
    pointer: usize,
    certificate: Option<(u64, u64)>,
}

pub struct Carry {
    entries: Vec<Entry>,
    bytes: Vec<u8>,
    order: Vec<usize>,
    order_entries: Vec<usize>,
    carried: Vec<usize>,
    pub(super) generation: u64,
    pub(super) version: u64,
    demand: Option<((u64, u64), (usize, usize))>,
    certified: Vec<bool>,
    certified_quiet: Option<(u64, u64)>,
    promote: bool,
    visits: Vec<Visit>,
    visits_failed: bool,
    producer: bool,
    pub(super) active: bool,
    changes: &'static dyn Changes,
    counts: Counts,
}

impl std::fmt::Debug for Carry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Carry")
            .field("entries", &self.entries.len())
            .field("carried", &self.carried.len())
            .field("generation", &self.generation)
            .finish()
    }
}

impl Default for Carry {
    fn default() -> Self {
        Self::with(&collector_observers::character_changes::Observer)
    }
}

impl Carry {
    pub(super) fn with(changes: &'static dyn Changes) -> Self {
        Self {
            entries: Vec::new(),
            bytes: Vec::new(),
            order: Vec::new(),
            order_entries: Vec::new(),
            carried: Vec::new(),
            generation: 0,
            version: fresh_version(),
            certified: Vec::new(),
            certified_quiet: None,
            demand: None,
            promote: false,
            visits: Vec::new(),
            visits_failed: false,
            producer: false,
            active: false,
            changes,
            counts: Counts::default(),
        }
    }

    #[cfg(test)]
    pub(super) fn counts(&self) -> Counts {
        self.counts
    }

    pub(super) fn allocated_bytes(&self) -> u64 {
        collector_primitives::raw_memory::vector_bytes(&self.entries)
            + collector_primitives::raw_memory::vector_bytes(&self.bytes)
            + collector_primitives::raw_memory::vector_bytes(&self.order)
            + collector_primitives::raw_memory::vector_bytes(&self.order_entries)
            + collector_primitives::raw_memory::vector_bytes(&self.carried)
            + collector_primitives::raw_memory::vector_bytes(&self.certified)
            + collector_primitives::raw_memory::vector_bytes(&self.visits)
    }

    fn position(&self, pointer: usize) -> Option<usize> {
        self.entries
            .binary_search_by_key(&pointer, |entry| entry.pointer)
            .ok()
    }

    fn position_at(&self, index: usize, pointer: usize) -> Option<usize> {
        match (self.order.get(index), self.order_entries.get(index)) {
            (Some(&held), Some(&entry)) if held == pointer => (entry != UNHELD).then_some(entry),
            _ => self.position(pointer),
        }
    }

    fn drop_carried(&mut self) {
        if !self.carried.is_empty() {
            self.carried.clear();
            self.generation = self.generation.wrapping_add(1);
        }
    }

    fn forget(&mut self) {
        self.version = fresh_version();
        self.entries.clear();
        self.bytes.clear();
        self.order.clear();
        self.order_entries.clear();
        self.certified.clear();
        self.promote = false;
        self.drop_carried();
    }

    pub(super) fn begin_hour(&mut self, active: bool) {
        self.active = active;
        self.producer = false;
        self.visits.clear();
        self.visits_failed = false;
        self.counts = Counts::default();
        if !active {
            self.drop_carried();
            return;
        }
        let changes = self.changes;
        let token = changes.quiet();
        let quiet = token.is_some()
            && token == self.certified_quiet
            && self.certified.len() == self.entries.len();
        if !quiet {
            self.certified.clear();
            self.certified_quiet = None;
        }
        let mut next = Vec::new();
        let prepared = engine_allocation::advisory(|| {
            engine_allocation::reserve_exact(&mut self.certified, self.entries.len())?;
            engine_allocation::reserve_exact(&mut next, self.entries.len())?;
            std::io::Result::Ok(())
        });
        if prepared.is_err() {
            self.forget();
            return;
        }
        if quiet {
            self.counts.quiet = true;
        } else {
            for entry in &self.entries {
                self.certified
                    .push(changes.sequence(entry.pointer) == Some(entry.certificate));
            }
            self.certified_quiet = token;
        }
        let promote = std::mem::take(&mut self.promote) || self.carried.is_empty();
        let mut carried = self.carried.iter().peekable();
        for (entry, &certified) in self.entries.iter().zip(&self.certified) {
            let held = carried.next_if_eq(&&entry.pointer).is_some();
            if certified && (promote || held) {
                next.push(entry.pointer);
            }
        }
        if next != self.carried {
            self.carried = next;
            self.generation = self.generation.wrapping_add(1);
        }
    }

    pub(super) fn certifies(&self, occurrences: &[ccharacter_manager::PortraitIdentity]) -> bool {
        self.active
            && self.certified.len() == self.entries.len()
            && occurrences.iter().enumerate().all(|(at, occurrence)| {
                self.position_at(at, occurrence.pointer)
                    .is_some_and(|index| {
                        self.certified[index] && self.entries[index].id == occurrence.character
                    })
            })
    }

    pub(super) fn carry_certified(&mut self) -> bool {
        fn certified<'a>(
            entries: &'a [Entry],
            flags: &'a [bool],
        ) -> impl Iterator<Item = usize> + 'a {
            entries
                .iter()
                .zip(flags)
                .filter(|&(_, &flag)| flag)
                .map(|(entry, _)| entry.pointer)
        }
        if self
            .carried
            .iter()
            .copied()
            .eq(certified(&self.entries, &self.certified))
        {
            return true;
        }
        let mut next = Vec::new();
        let count = certified(&self.entries, &self.certified).count();
        if engine_allocation::advisory(|| engine_allocation::reserve_exact(&mut next, count))
            .is_err()
        {
            return false;
        }
        next.extend(certified(&self.entries, &self.certified));
        self.carried = next;
        self.generation = self.generation.wrapping_add(1);
        true
    }

    pub(super) fn carries(&self, pointer: usize) -> bool {
        self.active && self.carried.binary_search(&pointer).is_ok()
    }

    pub(super) fn carried_row(&self, pointer: usize, id: u32) -> Option<(ccharacter::Row, &[u8])> {
        if !self.carries(pointer) {
            return None;
        }
        let entry = &self.entries[self.position(pointer)?];
        (entry.id == id).then(|| (entry.row, &self.bytes[entry.bytes.clone()]))
    }

    pub(super) fn carried_bytes(&self, pointer: usize) -> usize {
        self.text_len(self.position(pointer))
    }

    pub(super) fn census_demand(&mut self, census: super::census::Admitted<'_>) -> (usize, usize) {
        let key = (census.generation, self.version);
        if let Some((held, demand)) = self.demand
            && held == key
            && census.generation != 0
        {
            return demand;
        }
        let bytes = (0..census.len())
            .map(|index| self.text_len(self.position_at(index, census.occurrence(index).pointer)))
            .sum();
        let demand = (census.len(), bytes);
        self.demand = Some((key, demand));
        demand
    }

    fn text_len(&self, entry: Option<usize>) -> usize {
        entry.map_or(0, |index| self.entries[index].bytes.len())
    }

    pub(super) fn uncarry(&mut self, pointer: usize) {
        if let Ok(index) = self.carried.binary_search(&pointer) {
            self.carried.remove(index);
            self.generation = self.generation.wrapping_add(1);
        }
    }

    pub(super) fn visit(&mut self, pointer: usize, carried: bool) {
        let certificate = if carried {
            self.position(pointer)
                .map(|index| self.entries[index].certificate)
        } else {
            self.changes.sequence(pointer)
        };
        if engine_allocation::advisory(|| engine_allocation::reserve(&mut self.visits, 1)).is_err()
        {
            self.visits_failed = true;
            return;
        }
        self.visits.push(Visit {
            pointer,
            certificate,
        });
    }

    pub(super) fn finish_pass(&mut self, complete: bool) {
        self.producer = self.active && complete && !self.visits_failed;
    }

    pub(super) fn rebuild(
        &mut self,
        outcome: Outcome,
        rows: &[ccharacter::Row],
        bytes: &[u8],
        quiet: bool,
    ) {
        if !self.active {
            return;
        }
        let rows = match outcome {
            Outcome::Rows { len } => rows.get(..len),
            _ => None,
        };
        let Some(rows) = rows else {
            self.forget();
            collector_observers::character_changes::record(Table::Characters, &self.counts);
            return;
        };
        if quiet {
            self.counts.reused = rows.len() as u64;
            self.counts.read = 0;
            collector_observers::character_changes::record(Table::Characters, &self.counts);
            return;
        }
        let carried = rows
            .iter()
            .zip(self.order_for(rows.len()))
            .filter(|&(_, pointer)| self.carried.binary_search(&pointer).is_ok())
            .count();
        self.counts.reused = carried as u64;
        self.counts.read = (rows.len() - carried) as u64;
        if self.producer {
            if self.visits.len() == rows.len() {
                self.hold(rows, bytes);
            } else {
                self.forget();
            }
        }
        collector_observers::character_changes::record(Table::Characters, &self.counts);
    }

    fn order_for(&self, len: usize) -> impl Iterator<Item = usize> + '_ {
        let visits = self.producer.then_some(&self.visits);
        let held = (!self.producer && self.order.len() == len).then_some(&self.order);
        visits
            .into_iter()
            .flat_map(|visits| visits.iter().map(|visit| visit.pointer))
            .chain(held.into_iter().flat_map(|order| order.iter().copied()))
    }

    fn hold(&mut self, rows: &[ccharacter::Row], bytes: &[u8]) {
        let mut entries = Vec::new();
        let mut held = Vec::new();
        let mut order = Vec::new();
        let text: usize = rows
            .iter()
            .map(|row| row.name_len + row.token_len + row.template_file_len)
            .sum();
        let reserved = engine_allocation::advisory(|| {
            engine_allocation::reserve_exact(&mut entries, rows.len())?;
            engine_allocation::reserve_exact(&mut held, text)?;
            engine_allocation::reserve_exact(&mut order, rows.len())?;
            std::io::Result::Ok(())
        });
        if reserved.is_err() {
            self.forget();
            return;
        }
        let mut newly = 0_usize;
        for (row, visit) in rows.iter().zip(&self.visits) {
            order.push(visit.pointer);
            let Some(certificate) = visit.certificate else {
                continue;
            };
            let start = held.len();
            let mut row = *row;
            for (offset, len) in [
                (&mut row.name_start, row.name_len),
                (&mut row.token_start, row.token_len),
                (&mut row.template_file_start, row.template_file_len),
            ] {
                let local = held.len() - start;
                held.extend_from_slice(&bytes[*offset..*offset + len]);
                *offset = local;
            }
            if self.carried.binary_search(&visit.pointer).is_err() {
                newly += 1;
            }
            entries.push(Entry {
                pointer: visit.pointer,
                id: row.id,
                certificate,
                row,
                bytes: start..held.len(),
            });
        }
        super::sort_dropping_repeated(&mut entries, |entry| entry.pointer);
        let mut order_entries = Vec::new();
        if engine_allocation::advisory(|| {
            engine_allocation::reserve_exact(&mut order_entries, order.len())
        })
        .is_err()
        {
            self.forget();
            return;
        }
        self.entries = entries;
        self.version = fresh_version();
        order_entries.extend(
            order
                .iter()
                .map(|&pointer| self.position(pointer).unwrap_or(UNHELD)),
        );
        self.bytes = held;
        self.order = order;
        self.order_entries = order_entries;
        self.certified.clear();
        self.promote =
            newly != 0 && (self.carried.is_empty() || newly.saturating_mul(32) >= rows.len());
        let entries = &self.entries;
        let before = self.carried.len();
        self.carried
            .retain(|pointer| entries.binary_search_by_key(pointer, |e| e.pointer).is_ok());
        if self.carried.len() != before {
            self.generation = self.generation.wrapping_add(1);
        }
    }
}

fn fresh_version() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

#[cfg(test)]
mod order_tests {
    use super::*;

    #[test]
    fn held_positions_find_their_entries_and_moved_ones_search() {
        let _globals = crate::extended::characters::test_globals();
        let mut carry = Carry {
            active: true,
            ..Carry::default()
        };
        let pointers = [0x3000, 0x1000, 0x2000];
        carry.visits = (0..3)
            .map(|index| Visit {
                pointer: pointers[index],
                certificate: (index != 2).then_some((1, index as u64)),
            })
            .collect();
        let rows: Vec<_> = (0..3)
            .map(|index| ccharacter::Row {
                id: 10 + index,
                ..Default::default()
            })
            .collect();
        carry.hold(&rows, &[]);
        for (index, pointer) in pointers.into_iter().enumerate() {
            assert_eq!(carry.position_at(index, pointer), carry.position(pointer));
        }
        assert_eq!(carry.entries[carry.position_at(0, 0x3000).unwrap()].id, 10);
        assert_eq!(carry.entries[carry.position_at(1, 0x1000).unwrap()].id, 11);
        assert_eq!(carry.position_at(2, 0x2000), None);
        assert_eq!(carry.entries[carry.position_at(1, 0x3000).unwrap()].id, 10);
        assert_eq!(carry.entries[carry.position_at(7, 0x1000).unwrap()].id, 11);
        assert_eq!(carry.position_at(0, 0x2000), None);
    }
}
