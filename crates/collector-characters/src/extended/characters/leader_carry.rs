use super::*;
use collector_observers::unit_leader_changes::{Changes, Counts};
use std::ops::Range;

pub(super) trait Held: Copy + PartialEq + std::fmt::Debug {
    fn gather(&self, bytes: &[u8], text: &mut Vec<u8>, start: usize) -> Option<Self>;
    fn text_len(&self) -> usize;
    fn can_hold(&self) -> bool {
        true
    }
}

fn take(
    bytes: &[u8],
    (at, len): (usize, usize),
    text: &mut Vec<u8>,
    start: usize,
) -> Option<(usize, usize)> {
    let moved = text.len() - start;
    text.extend_from_slice(bytes.get(at..at.checked_add(len)?)?);
    Some((moved, len))
}

impl Held for LeaderRow {
    fn can_hold(&self) -> bool {
        self.training_reason.is_none()
    }
    fn gather(&self, bytes: &[u8], text: &mut Vec<u8>, start: usize) -> Option<Self> {
        let (text_a_start, _) = take(bytes, (self.text_a_start, self.text_a_len), text, start)?;
        let (text_b_start, _) = take(bytes, (self.text_b_start, self.text_b_len), text, start)?;
        let codename = if let Some(mut name) = self.codename {
            name.custom = take(bytes, name.custom, text, start)?;
            name.group = take(bytes, name.group, text, start)?;
            Some(name)
        } else {
            None
        };
        let training = take(bytes, self.training, text, start)?;
        let training_factors = take(bytes, self.training_factors, text, start)?;
        Some(LeaderRow {
            training,
            training_factors,
            text_a_start,
            text_b_start,
            codename,
            ..*self
        })
    }

    fn text_len(&self) -> usize {
        self.text_a_len
            + self.text_b_len
            + self.codename.map_or(0, |name| name.custom.1 + name.group.1)
            + self.training.1
            + self.training_factors.1
    }
}

impl Held for cunit_leader::TraitRow {
    fn gather(&self, bytes: &[u8], text: &mut Vec<u8>, start: usize) -> Option<Self> {
        let (token_start, _) = take(bytes, (self.token_start, self.token_len), text, start)?;
        Some(Self {
            token_start,
            ..*self
        })
    }

    fn text_len(&self) -> usize {
        self.token_len
    }
}

#[derive(Debug, Clone)]
struct Entry {
    pointer: usize,
    character: u32,
    leader: u32,
    certificate: (u64, u64),
    rows: Range<usize>,
    text: Range<usize>,
    carried: bool,
    seen: bool,
}

#[derive(Clone, Copy, Debug)]
struct Visit {
    index: usize,
    pointer: usize,
    character: u32,
    certificate: Option<(u64, u64)>,
    entry: Option<usize>,
    carryable: bool,
}

#[derive(Clone, Copy)]
pub(super) struct Family {
    pub record: fn(&Counts, usize),
}

pub struct LeaderCarry<R> {
    changes: &'static dyn Changes,
    family: Family,
    entries: Vec<Entry>,
    rows: Vec<R>,
    text: Vec<u8>,
    visits: Vec<Visit>,
    added: Vec<Entry>,
    added_rows: Vec<R>,
    added_text: Vec<u8>,
    token: Option<((u64, u64), u64)>,
    pending: Option<((u64, u64), u64)>,
    active: bool,
    layout: u64,
    carried: (usize, usize),
    counts: Counts,
}

pub(super) type UnitCarry = LeaderCarry<LeaderRow>;
pub(super) type TraitCarry = LeaderCarry<cunit_leader::TraitRow>;

impl<R> std::fmt::Debug for LeaderCarry<R> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LeaderCarry")
            .field("entries", &self.entries.len())
            .field("active", &self.active)
            .finish()
    }
}

pub(super) const UNITS: Family = Family {
    record: collector_observers::unit_leader_changes::record,
};

pub(super) const TRAITS: Family = Family {
    record: collector_observers::unit_leader_trait_changes::record,
};

impl Default for UnitCarry {
    fn default() -> Self {
        Self::with(&collector_observers::unit_leader_changes::Observer, UNITS)
    }
}

impl Default for TraitCarry {
    fn default() -> Self {
        Self::with(
            &collector_observers::unit_leader_trait_changes::Observer,
            TRAITS,
        )
    }
}

impl<R: Held> LeaderCarry<R> {
    pub(super) fn with(changes: &'static dyn Changes, family: Family) -> Self {
        Self {
            changes,
            family,
            entries: Vec::new(),
            rows: Vec::new(),
            text: Vec::new(),
            visits: Vec::new(),
            added: Vec::new(),
            added_rows: Vec::new(),
            added_text: Vec::new(),
            token: None,
            pending: None,
            active: false,
            layout: fresh_layout(),
            carried: (0, 0),
            counts: Counts::default(),
        }
    }

    pub(super) fn allocated_bytes(&self) -> u64 {
        use collector_primitives::raw_memory::vector_bytes;
        vector_bytes(&self.entries)
            + vector_bytes(&self.rows)
            + vector_bytes(&self.text)
            + vector_bytes(&self.visits)
            + vector_bytes(&self.added)
            + vector_bytes(&self.added_rows)
            + vector_bytes(&self.added_text)
    }

    #[cfg(test)]
    pub(super) fn counts(&self) -> Counts {
        self.counts
    }

    pub(super) fn begin_hour(&mut self) {
        self.active = false;
        self.carried = (0, 0);
        self.counts = Counts::default();
    }

    pub(super) fn certify<I: Iterator<Item = (usize, usize, u32)>>(
        &mut self,
        generation: u64,
        bound: usize,
        occurrences: impl FnOnce() -> I,
    ) -> bool {
        let token = self.changes.quiet().map(|quiet| (quiet, generation));
        let unchanged = generation != 0 && token.is_some() && token == self.token;
        self.token = None;
        self.pending = token;
        if !unchanged {
            self.visits.clear();
            let reserved =
                engine_allocation::advisory(|| engine_allocation::reserve(&mut self.visits, bound));
            if reserved.is_err() {
                return false;
            }
            for (index, pointer, character) in occurrences() {
                let entry = self.position(pointer);
                let certificate = self.changes.sequence(pointer);
                let carryable = entry.is_some_and(|at| {
                    let entry = &self.entries[at];
                    entry.character == character && Some(entry.certificate) == certificate
                });
                self.visits.push(Visit {
                    index,
                    pointer,
                    character,
                    certificate,
                    entry,
                    carryable,
                });
            }
        }
        let mut carrying = false;
        for visit in &self.visits {
            let Some(at) = visit.entry else {
                continue;
            };
            let entry = &mut self.entries[at];
            entry.carried = visit.carryable;
            if entry.carried {
                carrying = true;
                self.carried.0 += entry.rows.len();
                self.carried.1 += entry.text.len();
            }
        }
        self.active = true;
        carrying
    }

    pub(super) fn certified(&self) -> bool {
        self.active
    }

    pub(super) fn visit_count(&self) -> usize {
        self.visits.len()
    }

    pub(super) fn fresh_visit(&self, at: usize) -> Option<(usize, u32)> {
        let visit = self.visits.get(at)?;
        visit
            .entry
            .is_none_or(|entry| !self.entries[entry].carried)
            .then_some((visit.pointer, visit.character))
    }

    pub(super) fn visited(&self) -> Option<impl Iterator<Item = usize> + '_> {
        self.active
            .then(|| self.visits.iter().map(|visit| visit.index))
    }

    fn position(&self, pointer: usize) -> Option<usize> {
        self.entries
            .binary_search_by_key(&pointer, |entry| entry.pointer)
            .ok()
    }

    fn carried_entry(&self, pointer: usize) -> Option<&Entry> {
        let entry = &self.entries[self.position(pointer)?];
        (self.active && entry.carried).then_some(entry)
    }

    pub(super) fn carries(&self, pointer: usize) -> bool {
        self.carried_entry(pointer).is_some()
    }

    pub(super) fn carried_demand(&self) -> (usize, usize) {
        self.carried
    }

    pub(super) fn carried_rows(&self, pointer: usize) -> Option<(&[R], &[u8])> {
        self.carried_entry(pointer).map(|entry| {
            (
                &self.rows[entry.rows.clone()],
                &self.text[entry.text.clone()],
            )
        })
    }

    pub(super) fn table_key(&self, generation: u64) -> Option<(u64, u64)> {
        (generation != 0 && self.visits.iter().all(|visit| visit.entry.is_some()))
            .then_some((generation, self.layout))
    }

    pub(super) fn held_equals(&self, at: usize, rows: &[R], bytes: &[u8]) -> bool {
        let Some(entry) = self.visits.get(at).and_then(|visit| visit.entry) else {
            return false;
        };
        let entry = &self.entries[entry];
        if rows.iter().any(|row| !row.can_hold()) {
            return false;
        }
        let mut text = Vec::new();
        let gathered = rows.iter().map(|row| row.gather(bytes, &mut text, 0));
        self.rows[entry.rows.clone()]
            .iter()
            .copied()
            .map(Some)
            .eq(gathered)
            && self.text[entry.text.clone()] == text[..]
    }

    pub(super) fn forget(&mut self) {
        self.layout = fresh_layout();
        self.visits.clear();
        self.entries.clear();
        self.rows.clear();
        self.text.clear();
        self.token = None;
    }

    pub(super) fn rebuild(
        &mut self,
        outcome: Outcome,
        rows: &[R],
        bytes: &[u8],
        fresh: impl FnMut(&[R]) -> Option<(usize, u32)>,
    ) {
        if !std::mem::take(&mut self.active) {
            return;
        }
        let rows = match outcome {
            Outcome::Rows { len } => rows.get(..len),
            _ => None,
        };
        if rows.is_none_or(|rows| self.hold(rows, bytes, fresh).is_err()) {
            self.forget();
            return;
        }
        self.token = self.pending.take();
        (self.family.record)(&self.counts, self.visits.len());
    }

    fn hold(
        &mut self,
        rows: &[R],
        bytes: &[u8],
        mut fresh: impl FnMut(&[R]) -> Option<(usize, u32)>,
    ) -> Result<(), ()> {
        for entry in &mut self.entries {
            entry.seen = false;
        }
        self.added.clear();
        self.added_rows.clear();
        self.added_text.clear();
        let mut cursor = 0;
        for visit in &mut self.visits {
            if let Some(index) = visit.entry.filter(|&index| self.entries[index].carried) {
                let entry = &mut self.entries[index];
                entry.seen = true;
                cursor += entry.rows.len();
                self.counts.reused += 1;
                continue;
            }
            self.counts.read += 1;
            let rest = rows.get(cursor..).ok_or(())?;
            let (len, leader) = fresh(rest).ok_or(())?;
            let read = rest.get(..len).ok_or(())?;
            cursor += len;
            if read.iter().any(|row| !row.can_hold()) {
                if let Some(index) = visit.entry {
                    self.entries[index].seen = false;
                }
                visit.carryable = false;
                continue;
            }
            let held = visit.entry.map(|index| &self.entries[index]);
            let (rows_start, text_start) = (self.added_rows.len(), self.added_text.len());
            engine_allocation::advisory(|| {
                engine_allocation::reserve(&mut self.added_rows, len)?;
                engine_allocation::reserve(
                    &mut self.added_text,
                    read.iter().map(Held::text_len).sum(),
                )
            })
            .map_err(|_| ())?;
            for row in read {
                let gathered = row
                    .gather(bytes, &mut self.added_text, text_start)
                    .ok_or(())?;
                self.added_rows.push(gathered);
            }
            let same = held.is_some_and(|entry| {
                entry.leader == leader
                    && self.rows[entry.rows.clone()] == self.added_rows[rows_start..]
                    && self.text[entry.text.clone()] == self.added_text[text_start..]
            });
            let Some(certificate) = visit.certificate else {
                self.added_rows.truncate(rows_start);
                self.added_text.truncate(text_start);
                continue;
            };
            if same {
                let entry = &mut self.entries[visit.entry.unwrap()];
                visit.carryable = true;
                entry.certificate = certificate;
                entry.character = visit.character;
                entry.seen = true;
                self.added_rows.truncate(rows_start);
                self.added_text.truncate(text_start);
                continue;
            }
            engine_allocation::advisory(|| engine_allocation::reserve(&mut self.added, 1))
                .map_err(|_| ())?;
            if let Some(index) = visit.entry {
                self.entries[index].seen = false;
            }
            self.added.push(Entry {
                pointer: visit.pointer,
                character: visit.character,
                leader,
                certificate,
                rows: rows_start..self.added_rows.len(),
                text: text_start..self.added_text.len(),
                carried: false,
                seen: true,
            });
        }
        if cursor != rows.len() {
            return Err(());
        }
        if self.added.is_empty() && self.entries.iter().all(|entry| entry.seen) {
            return Ok(());
        }
        self.restructure().map_err(|_| ())
    }

    fn restructure(&mut self) -> std::io::Result<()> {
        let kept = self.entries.iter().filter(|entry| entry.seen);
        let (entries, rows, text) = kept.clone().fold((0, 0, 0), |(e, r, t), entry| {
            (e + 1, r + entry.rows.len(), t + entry.text.len())
        });
        let mut next_entries = Vec::new();
        let mut next_rows = Vec::new();
        let mut next_text = Vec::new();
        engine_allocation::advisory(|| {
            engine_allocation::reserve_exact(&mut next_entries, entries + self.added.len())?;
            engine_allocation::reserve_exact(&mut next_rows, rows + self.added_rows.len())?;
            engine_allocation::reserve_exact(&mut next_text, text + self.added_text.len())
        })?;
        let mut push = |entry: &Entry, rows: &[R], text: &[u8]| {
            let (row_start, text_start) = (next_rows.len(), next_text.len());
            next_rows.extend_from_slice(&rows[entry.rows.clone()]);
            next_text.extend_from_slice(&text[entry.text.clone()]);
            next_entries.push(Entry {
                rows: row_start..next_rows.len(),
                text: text_start..next_text.len(),
                carried: false,
                seen: false,
                ..entry.clone()
            });
        };
        for entry in kept {
            push(entry, &self.rows, &self.text);
        }
        for entry in &self.added {
            push(entry, &self.added_rows, &self.added_text);
        }
        next_entries.sort_unstable_by_key(|entry| entry.pointer);
        next_entries.dedup_by_key(|entry| entry.pointer);
        self.entries = next_entries;
        self.rows = next_rows;
        self.text = next_text;
        self.layout = fresh_layout();
        for visit in &mut self.visits {
            visit.entry = self
                .entries
                .binary_search_by_key(&visit.pointer, |entry| entry.pointer)
                .ok();
            visit.carryable = visit.entry.is_some();
        }
        Ok(())
    }
}

fn fresh_layout() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

pub(super) fn held_rows_into<R: Copy + Default>(
    held: &[R],
    text: &[u8],
    rows: &mut Vec<R>,
    at: usize,
    bytes: &mut Vec<u8>,
    from: usize,
) -> Result<super::Stored, &'static str> {
    engine_native_memory::walk::grow(rows, at + held.len())?;
    engine_native_memory::walk::grow(bytes, from + text.len())?;
    rows[at..at + held.len()].copy_from_slice(held);
    bytes[from..from + text.len()].copy_from_slice(text);
    Ok(super::Stored {
        rows: held.len(),
        bytes_len: text.len(),
        required_bytes: text.len(),
    })
}

pub(super) fn held_rows<R: Copy + Default>(
    held: &[R],
    text: &[u8],
    mut rows: engine_native_memory::walk::Buffer<'_, R>,
    mut bytes: engine_native_memory::walk::Buffer<'_, u8>,
) -> ReadResult {
    let fits = rows
        .ensure(held.len())
        .and_then(|rows| Ok(rows && bytes.ensure(text.len())?));
    let fits = match fits {
        Ok(fits) => fits,
        Err(reason) => return ReadResult::rows(Outcome::Unavailable { reason }),
    };
    if !fits {
        return ReadResult {
            outcome: Outcome::Truncated {
                len: 0,
                unread_rows: u32::try_from(held.len()).unwrap_or(u32::MAX),
            },
            bytes_len: 0,
            required_bytes: text.len(),
        };
    }
    rows[..held.len()].copy_from_slice(held);
    bytes[..text.len()].copy_from_slice(text);
    ReadResult {
        outcome: Outcome::Rows { len: held.len() },
        bytes_len: text.len(),
        required_bytes: text.len(),
    }
}
