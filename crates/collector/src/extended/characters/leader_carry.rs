//! Unit leader rows held between hours: UnitLeaders on quiet roster hours
//! ([`crate::unit_leader_changes`]) and UnitLeaderTraits in the traits job
//! ([`crate::unit_leader_trait_changes`]).
//!
//! A capture takes the certificate of each leader its role census names
//! before reading anything ([`LeaderCarry::certify`]). A leader is carried,
//! without reading it, when the job holds a read of it at the census's unit
//! pointer for the census's character under an equal certificate and its id
//! is outside the hour's slice; the capture reads only its other leaders and
//! the decode publishes the held rows. When the domain's quiet token and the
//! census generation are those taken before the certification whose rows the
//! job last held, no observed writer ran since and every occurrence is the
//! one certified then, so each held certificate still matches without a
//! lookup, and the occurrences with a unit leader are the ones it visited,
//! so the hour reuses that certification's visits and costs one pass over
//! its leaders. The decode worker compares each fresh read of a carryable leader
//! (the slice's, every one in verify mode) with its held rows, poisoning reuse
//! at a difference ([`LeaderCarry::rebuild`]). A fresh read equal to its held
//! rows only renews the entry's certificate; the held rows are rebuilt only
//! when a leader's rows changed or a leader appeared or left the census. A
//! row follows from its leader's fields alone: UnitLeaders' skill block is
//! the shared definition and a trait row's definition fields are the
//! content's, which only a load replaces. Captures that do not certify carry
//! and hold nothing.
use super::*;
use crate::{
    sampled_verify::Check,
    unit_leader_changes::{Changes, Counts},
};
use std::ops::Range;

/// A held row type: its text spans, relative to its leader's held text.
pub(super) trait Held: Copy + PartialEq + std::fmt::Debug {
    /// Appends this fresh row's text from the table's `bytes` to `text` and
    /// returns the row with spans from `start`.
    fn gather(&self, bytes: &[u8], text: &mut Vec<u8>, start: usize) -> Self;
    /// The row's text bytes.
    fn text_len(&self) -> usize;
    /// A refused appended value must refresh only this leader, without
    /// discarding unrelated leaders' certified rows.
    fn can_hold(&self) -> bool {
        true
    }
}

/// Appends `bytes[at..at + len]` to `text`; the span from `start`.
fn take(
    bytes: &[u8],
    (at, len): (usize, usize),
    text: &mut Vec<u8>,
    start: usize,
) -> (usize, usize) {
    let moved = text.len() - start;
    text.extend_from_slice(&bytes[at..at + len]);
    (moved, len)
}

impl Held for LeaderRow {
    fn can_hold(&self) -> bool {
        self.training_reason.is_none()
    }
    fn gather(&self, bytes: &[u8], text: &mut Vec<u8>, start: usize) -> Self {
        let (text_a_start, _) = take(bytes, (self.text_a_start, self.text_a_len), text, start);
        let (text_b_start, _) = take(bytes, (self.text_b_start, self.text_b_len), text, start);
        let codename = self.codename.map(|mut name| {
            name.custom = take(bytes, name.custom, text, start);
            name.group = take(bytes, name.group, text, start);
            name
        });
        let training = take(bytes, self.training, text, start);
        let training_factors = take(bytes, self.training_factors, text, start);
        LeaderRow {
            training,
            training_factors,
            text_a_start,
            text_b_start,
            codename,
            ..*self
        }
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
    fn gather(&self, bytes: &[u8], text: &mut Vec<u8>, start: usize) -> Self {
        let (token_start, _) = take(bytes, (self.token_start, self.token_len), text, start);
        Self {
            token_start,
            ..*self
        }
    }

    fn text_len(&self) -> usize {
        self.token_len
    }
}

/// One held read.
#[derive(Debug, Clone)]
struct Entry {
    pointer: usize,
    character: u32,
    /// The leader's id: the slice key.
    leader: u32,
    certificate: (u64, u64),
    rows: Range<usize>,
    text: Range<usize>,
    /// Carried this hour.
    carried: bool,
    /// The census named it this hour.
    seen: bool,
}

/// One census occurrence with a unit leader, in census order.
#[derive(Clone, Copy, Debug)]
struct Visit {
    /// The occurrence's census index.
    index: usize,
    pointer: usize,
    character: u32,
    /// Taken before any read of the hour; `None` when unobserved.
    certificate: Option<(u64, u64)>,
    /// The held entry at its pointer.
    entry: Option<usize>,
    /// That entry's certificate and character still match.
    carryable: bool,
}

/// The family a carry reports to.
#[derive(Clone, Copy)]
pub(super) struct Family {
    /// The family's `*_verify` feature.
    pub verify: bool,
    /// Publishes one capture's counts and the leaders it visited.
    pub record: fn(&Counts, usize),
}

pub(crate) struct LeaderCarry<R> {
    changes: &'static dyn Changes,
    family: Family,
    /// Tests: the slice; production follows the hour.
    check: Option<Check>,
    /// Sorted by pointer.
    entries: Vec<Entry>,
    rows: Vec<R>,
    text: Vec<u8>,
    visits: Vec<Visit>,
    /// Fresh reads that replace or add an entry, with their rows and text.
    added: Vec<Entry>,
    added_rows: Vec<R>,
    added_text: Vec<u8>,
    /// The quiet token and census generation taken before the certification
    /// whose rows the job holds; every held certificate was current then.
    token: Option<((u64, u64), u64)>,
    /// Those of this hour's certification, until its rows are held.
    pending: Option<((u64, u64), u64)>,
    /// A capture certified this payload's leaders.
    active: bool,
    /// Names the held entries and their order: a fresh value whenever they
    /// are rebuilt or forgotten.
    layout: u64,
    /// Rows and text bytes this hour's carried leaders publish.
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

/// UnitLeaders on quiet roster hours.
pub(super) const UNITS: Family = Family {
    verify: crate::unit_leader_changes::VERIFY,
    record: crate::unit_leader_changes::record,
};

/// UnitLeaderTraits in the traits job.
pub(super) const TRAITS: Family = Family {
    verify: crate::unit_leader_trait_changes::VERIFY,
    record: crate::unit_leader_trait_changes::record,
};

impl Default for UnitCarry {
    fn default() -> Self {
        Self::with(&crate::unit_leader_changes::Observer, UNITS, None)
    }
}

impl Default for TraitCarry {
    fn default() -> Self {
        Self::with(&crate::unit_leader_trait_changes::Observer, TRAITS, None)
    }
}

impl<R: Held> LeaderCarry<R> {
    pub(super) fn with(
        changes: &'static dyn Changes,
        family: Family,
        check: Option<Check>,
    ) -> Self {
        Self {
            changes,
            family,
            check,
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
        use crate::raw_memory::vector_bytes;
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

    #[cfg(test)]
    pub(super) fn set_check(&mut self, check: Check) {
        self.check = Some(check);
    }

    /// Each capture, before anything else: nothing is certified yet.
    pub(super) fn begin_hour(&mut self) {
        self.active = false;
        self.carried = (0, 0);
        self.counts = Counts::default();
    }

    /// Copy side, before any leader read: certify the `(census index, unit
    /// pointer, character id)` occurrences with a unit leader, at most
    /// `bound`, of the census with `generation`, in order. Returns whether
    /// any leader is carried.
    pub(super) fn certify<I: Iterator<Item = (usize, usize, u32)>>(
        &mut self,
        generation: u64,
        bound: usize,
        occurrences: impl FnOnce() -> I,
    ) -> bool {
        let token = self.changes.quiet().map(|quiet| (quiet, generation));
        let unchanged = generation != 0 && token.is_some() && token == self.token;
        // Until this hour's rows are held, a held certificate may be one this
        // hour's lookups found stale.
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
        // Unchanged, the visits are the held certification's: each held
        // entry's certificate is the one its read was held under.
        let check = self
            .check
            .unwrap_or_else(|| crate::sampled_verify::current_burned_in(self.family.verify));
        let mut carrying = false;
        for visit in &self.visits {
            let Some(at) = visit.entry else {
                continue;
            };
            let entry = &mut self.entries[at];
            entry.carried = visit.carryable && !check.covers(u64::from(entry.leader));
            if entry.carried {
                carrying = true;
                self.carried.0 += entry.rows.len();
                self.carried.1 += entry.text.len();
            }
        }
        self.active = true;
        carrying
    }

    /// Whether a capture certified this payload's leaders.
    pub(super) fn certified(&self) -> bool {
        self.active
    }

    /// Copy side: this hour's visits, in census order.
    pub(super) fn visit_count(&self) -> usize {
        self.visits.len()
    }

    /// Copy side: the unit pointer and character id of visit `at` when this
    /// hour reads its leader.
    pub(super) fn fresh_visit(&self, at: usize) -> Option<(usize, u32)> {
        let visit = self.visits.get(at)?;
        visit
            .entry
            .is_none_or(|entry| !self.entries[entry].carried)
            .then_some((visit.pointer, visit.character))
    }

    /// Decode: the census indices of this payload's certified leaders, in
    /// census order; `None` when no capture certified them.
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

    /// Copy side: whether `pointer`'s leader is carried this hour.
    pub(super) fn carries(&self, pointer: usize) -> bool {
        self.carried_entry(pointer).is_some()
    }

    /// Copy side: the rows and text bytes this hour's carried leaders publish.
    pub(super) fn carried_demand(&self) -> (usize, usize) {
        self.carried
    }

    /// Decode: a carried leader's held rows and their text, spans from zero.
    pub(super) fn carried_rows(&self, pointer: usize) -> Option<(&[R], &[u8])> {
        self.carried_entry(pointer).map(|entry| {
            (
                &self.rows[entry.rows.clone()],
                &self.text[entry.text.clone()],
            )
        })
    }

    /// The key of the table the held entries make for the census with
    /// `generation`: every visit's held rows in visit order. The layout names
    /// the entries, so equal keys name equal tables. `None` when a visit
    /// holds none.
    pub(super) fn table_key(&self, generation: u64) -> Option<(u64, u64)> {
        (generation != 0 && self.visits.iter().all(|visit| visit.entry.is_some()))
            .then_some((generation, self.layout))
    }

    /// Decode: whether visit `at`'s fresh rows, with spans into `bytes`,
    /// equal its held rows.
    pub(super) fn held_equals(&self, at: usize, rows: &[R], bytes: &[u8]) -> bool {
        let Some(entry) = self.visits.get(at).and_then(|visit| visit.entry) else {
            return false;
        };
        let entry = &self.entries[entry];
        let mut text = Vec::new();
        let gathered = rows.iter().map(|row| row.gather(bytes, &mut text, 0));
        self.rows[entry.rows.clone()].iter().copied().eq(gathered)
            && self.text[entry.text.clone()] == text[..]
    }

    fn forget(&mut self) {
        self.layout = fresh_layout();
        self.visits.clear();
        self.entries.clear();
        self.rows.clear();
        self.text.clear();
        self.token = None;
    }

    /// Decode worker, after the payload's rows: compare the fresh reads of
    /// carryable leaders, renew or hold every fresh read the census named and
    /// drop the leaders it no longer names. `fresh` gives each fresh visit's
    /// row count and leader id from the rows that remain, in visit order;
    /// carried visits publish their held rows.
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
        if self.counts.mismatches != 0 {
            self.changes.mismatch();
        }
        (self.family.record)(&self.counts, self.visits.len());
    }

    /// Storage is advisory: an error forgets every held read.
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
            // The fresh read's rows and text, appended to the added buffers.
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
                let gathered = row.gather(bytes, &mut self.added_text, text_start);
                self.added_rows.push(gathered);
            }
            let same = held.is_some_and(|entry| {
                entry.leader == leader
                    && self.rows[entry.rows.clone()] == self.added_rows[rows_start..]
                    && self.text[entry.text.clone()] == self.added_text[text_start..]
            });
            if visit.carryable {
                self.counts.verified += 1;
                self.counts.mismatches += u64::from(!same);
            }
            let Some(certificate) = visit.certificate else {
                // Unobserved: the census names it, but nothing certifies a
                // held read.
                self.added_rows.truncate(rows_start);
                self.added_text.truncate(text_start);
                continue;
            };
            if same {
                // Renewed in place.
                let entry = &mut self.entries[visit.entry.unwrap()];
                // The next unchanged hour's visit is carryable again.
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
                // Replaced by the fresh read.
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

    /// Rebuilds the held buffers from the seen entries and the added reads.
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
        // The visits name the rebuilt entries, and a held read's visit is
        // carryable under the certificate it was held with.
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

/// A layout no other carry has held.
fn fresh_layout() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Appends one carried leader's held rows, whose spans start at zero, and
/// their text as a walk stores them.
pub(super) fn held_rows<R: Copy>(
    held: &[R],
    text: &[u8],
    rows: &mut [R],
    bytes: &mut [u8],
) -> ReadResult {
    if rows.len() < held.len() || bytes.len() < text.len() {
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
