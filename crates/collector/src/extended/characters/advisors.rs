//! Advisor definitions retained only under uninterrupted mutation observation.
//! Double-buffered owned arenas let one changed character grow without moving
//! later cache keys or forcing the rest of the roster to be reacquired.
use super::*;
use std::cell::Cell;
#[cfg(feature = "advisor_profile")]
pub(crate) mod profile;

pub(crate) trait Changes {
    fn quiet(&self) -> Option<(u64, u64)>;
    fn revision(&self, map: usize) -> Option<(u64, u64)>;
    fn verify(&self) -> bool;
}
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Counts {
    pub planned: u64,
    pub reused: u64,
    pub read: u64,
    pub roles: u64,
    pub verified: u64,
    pub mismatches: u64,
}
#[derive(Clone, Debug)]
struct Entry {
    pointer: usize,
    id: u32,
    country_offset: Option<usize>,
    header: [u8; 16],
    sentinel: [u8; 26],
    revision: Option<(u64, u64)>,
    rows: std::ops::Range<usize>,
    bytes: std::ops::Range<usize>,
}
#[derive(Debug)]
struct Owned {
    rows: Vec<cadvisor::Row>,
    bytes: Vec<u8>,
    entries: Vec<Entry>,
}
impl Owned {
    fn new(rows: usize, bytes: usize, characters: usize) -> Self {
        Self {
            rows: vec![cadvisor::Row::default(); rows],
            bytes: vec![0; bytes],
            entries: Vec::with_capacity(characters),
        }
    }
    fn allocated(&self) -> u64 {
        crate::raw_memory::vector_bytes(&self.rows)
            + crate::raw_memory::vector_bytes(&self.bytes)
            + crate::raw_memory::vector_bytes(&self.entries)
    }
}
#[derive(Debug)]
struct Section {
    plan: crate::snapshot_plan::SnapshotPlan,
    geometry: (usize, u32),
    #[cfg(feature = "advisor_profile")]
    diagnostic: [u64; 5],
    current: Owned,
    previous: Owned,
    valid: bool,
    scope: (usize, usize),
    observation: Option<(u64, u64)>,
    pub(super) result: ReadResult,
    pub(super) raw_bytes: u64,
    pub(super) counts: Counts,
    characters: usize,
    charge: crate::raw_memory::Charge,
}
impl Section {
    fn with_capacity(rows: usize, bytes: usize, characters: usize) -> Self {
        let mut cache = Self {
            plan: crate::snapshot_plan::SnapshotPlan::new(characters * 6, characters * 96),
            geometry: (0, 0),
            #[cfg(feature = "advisor_profile")]
            diagnostic: [0; 5],
            current: Owned::new(rows, bytes, characters),
            previous: Owned::new(rows, bytes, characters),
            valid: false,
            scope: (0, 0),
            observation: None,
            result: ReadResult::rows(Outcome::Rows { len: 0 }),
            raw_bytes: 0,
            counts: Counts::default(),
            characters: 0,
            charge: Default::default(),
        };
        cache.update_charge();
        cache
    }
    fn update_charge(&mut self) {
        self.charge.set(crate::raw_memory::Capacity {
            payload_vectors: self.current.allocated() + self.previous.allocated(),
            auxiliary_vectors: self.plan.allocated_bytes() as u64,
            ..Default::default()
        });
    }
    pub fn needs_growth(&self) -> bool {
        matches!(self.result.outcome, Outcome::Truncated { .. })
            || self.characters > self.current.entries.capacity()
    }
    pub fn grow(&mut self) {
        let (rows, bytes) = match self.result.outcome {
            Outcome::Truncated { len, unread_rows } => (
                self.current
                    .rows
                    .len()
                    .saturating_mul(2)
                    .max(len.saturating_add(unread_rows as usize)),
                self.current
                    .bytes
                    .len()
                    .saturating_mul(2)
                    .max(self.result.required_bytes),
            ),
            _ => (self.current.rows.len(), self.current.bytes.len()),
        };
        for owned in [&mut self.current, &mut self.previous] {
            owned.rows.resize(rows, cadvisor::Row::default());
            owned.bytes.resize(bytes, 0);
            owned
                .entries
                .reserve(self.characters.saturating_sub(owned.entries.len()));
        }
        self.update_charge();
    }
    #[cfg(test)]
    pub fn decode(&self, captured: &mut Captured) {
        let rows = self.result.outcome.len();
        // Decode owns these allocations. Acquisition never resizes an arena.
        if let Err(reason) = captured.reserve_advisors(AdvisorDemand {
            rows,
            bytes: self.result.bytes_len,
            overflowed: false,
        }) {
            captured
                .advisors
                .primary(ReadResult::rows(Outcome::Unavailable { reason }));
        } else {
            captured.advisors.rows.rows[..rows].copy_from_slice(&self.current.rows[..rows]);
            captured.advisors.bytes.first[..self.result.bytes_len]
                .copy_from_slice(&self.current.bytes[..self.result.bytes_len]);
            captured.advisors.primary(self.result);
        }
    }
    fn prepare(&mut self) {
        // Country is a result value, never a selecting pointer or a validation
        // predicate. Bind its exact read occurrence to the matching character;
        // the remaining observations retain equality controls.
        let data_reads = self.plan.pending_reads().map(|reads| {
            let mut countries = reads.iter().enumerate().filter(|(_, read)| read.len == 4);
            let mut indexes = Vec::with_capacity(self.current.entries.len());
            for entry in &mut self.current.entries {
                let (index, read) = countries.next()?;
                if entry.pointer.checked_add(ccharacter::COUNTRY) != Some(read.address) {
                    return None;
                }
                entry.country_offset = Some(read.destination_offset);
                indexes.push(index);
            }
            countries.next().is_none().then_some(indexes)
        });
        match data_reads {
            Some(Some(indexes)) => self.plan.prepare_with_data(&indexes),
            Some(None) => {
                // A changed reader shape retains full comparisons. Never
                // infer a binding from an address alone or omit an unknown read.
                for entry in &mut self.current.entries {
                    entry.country_offset = None;
                }
                self.plan.prepare();
            }
            None => self.plan.prepare(),
        }
        self.update_charge();
    }
    #[cfg(test)]
    fn capture(
        &mut self,
        memory: &impl Memory,
        context: CharactersCopyContext,
        observer: &dyn Changes,
    ) {
        let roster =
            ccharacter_manager::Roster::read(memory, context.base, context.game_state).unwrap();
        self.capture_section(
            memory,
            context,
            observer,
            roster,
            (0, roster.count()),
            false,
        );
    }
    fn capture_section(
        &mut self,
        memory: &impl Memory,
        context: CharactersCopyContext,
        observer: &dyn Changes,
        roster: ccharacter_manager::Roster,
        geometry: (usize, u32),
        count_only: bool,
    ) {
        let scope = (context.base, context.game_state);
        let before = observer.quiet();
        let counted = Counted::new(memory);
        let certified = self.valid
            && self.scope == scope
            && self.geometry == geometry
            && !count_only
            && !observer.verify()
            && before.is_some()
            && (self.observation == before
                || self.current.entries.iter().all(|entry| {
                    entry.revision.is_some()
                        && observer.revision(entry.pointer + ccharacter::ADVISOR_MAP_HEAD)
                            == entry.revision
                }));
        self.counts = Counts::default();
        if self.plan.begin(&counted, certified) {
            #[cfg(feature = "advisor_profile")]
            {
                self.diagnostic = counted.plan.get();
            }
            self.raw_bytes = counted.bytes.get();
            if before != observer.quiet() {
                self.valid = false;
                self.result = ReadResult::rows(Outcome::Unavailable {
                    reason: "advisor mutated during acquisition",
                });
                self.plan.invalidate();
            } else {
                for entry in &self.current.entries {
                    if let Some(offset) = entry.country_offset {
                        let Some(bytes) = self.plan.completed_bytes(offset, 4) else {
                            self.valid = false;
                            self.result = ReadResult::rows(Outcome::Unavailable {
                                reason: "advisor country binding unavailable",
                            });
                            self.plan.invalidate();
                            return;
                        };
                        let country = u32::from_le_bytes(bytes.try_into().unwrap());
                        let rows = &mut self.current.rows[entry.rows.clone()];
                        // Every role in this occurrence has the same country.
                        // Avoid making unchanged row cache lines writable on
                        // each validation pass (decode may have read them on
                        // another core since the previous acquisition).
                        if rows.first().is_some_and(|row| row.country != country) {
                            for row in rows {
                                row.country = country;
                            }
                        }
                    }
                }
                self.observation = before;
                self.counts.planned = 1;
                self.counts.reused = self.characters as u64;
                self.counts.roles = self.result.outcome.len() as u64;
            }
            return;
        }
        self.geometry = geometry;
        std::mem::swap(&mut self.current, &mut self.previous);
        self.current.entries.clear();
        self.counts = Counts::default();
        let reuse = self.valid && self.scope == scope && before.is_some();
        self.valid = false;
        self.scope = scope;
        let observed = self.plan.view(&counted);
        let prefetched = Prefetched {
            memory: &observed,
            geometry,
        };
        let memory = &prefetched;
        let mut batch = if count_only {
            Batch::new(&mut [], &mut [])
        } else {
            Batch::new(&mut self.current.rows, &mut self.current.bytes)
        };
        self.characters = roster.count() as usize;
        let result = (|| {
            let mut cursor = roster.cursor();
            let mut page = ccharacter_manager::ViewPage::default();
            let mut index = 0;
            while batch.active() {
                let view = loop {
                    if let Some(view) = page.next() {
                        break Some(view?);
                    }
                    if !cursor.refill_page(memory, &mut page, [false, false, false, true])? {
                        break None;
                    }
                };
                let Some(view) = view else {
                    break;
                };
                let character = view.entry()?;
                let map = cadvisor::MapView::read(&view, character.pointer)?;
                let previous = self.previous.entries.get(index);
                // A quiet whole-domain revision avoids one hash-table lookup
                // per unchanged character. Identity/controls remain fresh.
                let revision = if reuse
                    && self.observation == before
                    && previous.is_some_and(|entry| {
                        entry.pointer == character.pointer
                            && entry.id == character.id
                            && entry.revision.is_some()
                    }) {
                    previous.unwrap().revision
                } else {
                    observer.revision(character.pointer + ccharacter::ADVISOR_MAP_HEAD)
                };
                let hit = reuse
                    && batch.unread == 0
                    && previous.is_some_and(|entry| {
                        entry.pointer == character.pointer
                            && entry.id == character.id
                            && entry.header == map.header
                            && entry.sentinel == map.sentinel
                            && revision.is_some()
                            && entry.revision == revision
                            && entry.rows.len() <= batch.rows.len() - batch.len
                            && entry.bytes.len() <= batch.bytes.len() - batch.bytes_len
                    });
                let row_start = batch.len;
                let byte_start = batch.bytes_len;
                if hit && !observer.verify() {
                    let entry = previous.unwrap();
                    let rows = entry.rows.len();
                    let bytes = entry.bytes.len();
                    batch.bytes[byte_start..byte_start + bytes]
                        .copy_from_slice(&self.previous.bytes[entry.bytes.clone()]);
                    for (target, source) in batch.rows[row_start..row_start + rows]
                        .iter_mut()
                        .zip(&self.previous.rows[entry.rows.clone()])
                    {
                        *target = *source;
                        target.country = character.country;
                        target.key_start = source.key_start - entry.bytes.start + byte_start;
                        target.idea_start = source.idea_start - entry.bytes.start + byte_start;
                        target.slot_start = source.slot_start - entry.bytes.start + byte_start;
                    }
                    batch.len += rows;
                    batch.bytes_len += bytes;
                    batch.required_bytes = batch
                        .required_bytes
                        .checked_add(bytes)
                        .ok_or("character byte count overflow")?;
                    self.counts.reused += 1;
                    self.counts.roles += rows as u64;
                } else {
                    batch.append(
                        |rows, bytes| {
                            // Role payloads cannot select another character's
                            // controls. Their future reuse is certified by the
                            // watched revision, never by these omitted reads.
                            let _payload = self.plan.untracked_reads();
                            cadvisor::walk_entry(&map, context.base, character, rows, bytes).into()
                        },
                        |row, offset| {
                            relocate(&mut row.key_start, offset)
                                && relocate(&mut row.idea_start, offset)
                                && relocate(&mut row.slot_start, offset)
                        },
                    );
                    self.counts.read += 1;
                    if hit {
                        self.counts.verified += 1;
                        let old = previous.unwrap();
                        let same = batch.failure.is_none()
                            && batch.unread == 0
                            && batch.len - row_start == old.rows.len()
                            && batch.bytes[byte_start..batch.bytes_len]
                                == self.previous.bytes[old.bytes.clone()]
                            && batch.rows[row_start..batch.len]
                                .iter()
                                .zip(&self.previous.rows[old.rows.clone()])
                                .all(|(new, old_row)| {
                                    let mut old_row = *old_row;
                                    old_row.country = character.country;
                                    old_row.key_start =
                                        old_row.key_start - old.bytes.start + byte_start;
                                    old_row.idea_start =
                                        old_row.idea_start - old.bytes.start + byte_start;
                                    old_row.slot_start =
                                        old_row.slot_start - old.bytes.start + byte_start;
                                    *new == old_row
                                });
                        if !same {
                            self.counts.mismatches += 1;
                        }
                    }
                }
                if self.current.entries.len() < self.current.entries.capacity() {
                    self.current.entries.push(Entry {
                        pointer: character.pointer,
                        id: character.id,
                        country_offset: None,
                        header: map.header,
                        sentinel: map.sentinel,
                        revision,
                        rows: row_start..batch.len,
                        bytes: byte_start..batch.bytes_len,
                    });
                }
                index += 1;
            }
            Ok::<_, &'static str>(())
        })();
        if let Err(reason) = result {
            batch.fail(reason);
        }
        if before != observer.quiet() {
            batch.fail("advisor mutated during acquisition");
        }
        #[cfg(feature = "advisor_profile")]
        {
            self.diagnostic = counted.plan.get();
        }
        self.result = batch.result();
        if !self.plan.finish(
            matches!(self.result.outcome, Outcome::Rows { .. })
                && before.is_some()
                && self.current.entries.len() == self.characters
                && self
                    .current
                    .entries
                    .iter()
                    .all(|entry| entry.revision.is_some())
                && self.counts.mismatches == 0
                && !observer.verify(),
        ) {
            self.result = ReadResult::rows(Outcome::Unavailable {
                reason: "advisor plan prefix differs",
            });
        }
        self.raw_bytes = counted.bytes.get();
        self.valid = matches!(self.result.outcome, Outcome::Rows { .. })
            && before.is_some()
            && self.current.entries.len() == self.characters
            && self.counts.mismatches == 0;
        self.observation = before;
    }
}

/// Independent historical/generated certificates keep generated roster growth
/// from invalidating the large historical pass. Roots are acquired once first.
#[derive(Debug)]
pub(crate) struct Cache {
    sections: [Section; 2],
    pub(super) result: ReadResult,
    pub(super) raw_bytes: u64,
    pub(super) counts: Counts,
}
impl Cache {
    pub fn new() -> Self {
        Self {
            sections: [
                Section::with_capacity(16384, 1 << 20, 16384),
                Section::with_capacity(4096, 1 << 18, 8192),
            ],
            result: ReadResult::rows(Outcome::Rows { len: 0 }),
            raw_bytes: 0,
            counts: Counts::default(),
        }
    }
    pub fn needs_growth(&self) -> bool {
        self.sections.iter().any(Section::needs_growth)
    }
    pub fn grow(&mut self) {
        for section in &mut self.sections {
            section.grow();
        }
    }
    pub fn prepare(&mut self) {
        for section in &mut self.sections {
            section.prepare();
        }
    }
    pub fn capture(
        &mut self,
        memory: &impl Memory,
        context: CharactersCopyContext,
        observer: &dyn Changes,
    ) {
        self.counts = Counts::default();
        self.raw_bytes = 0;
        self.result = ReadResult::rows(Outcome::Rows { len: 0 });
        let counted = Counted::new(memory);
        let before = observer.quiet();
        #[cfg(feature = "advisor_profile")]
        let mut measurements = [[0; 11]; 2];
        #[cfg(feature = "advisor_profile")]
        let mut measurement_iter = measurements.iter_mut();
        let result = (|| {
            let roster =
                ccharacter_manager::Roster::read(&counted, context.base, context.game_state)?;
            let mut len = 0;
            let mut unread = 0_u32;
            for (section, (array, roster)) in self.sections.iter_mut().zip(roster.sections()) {
                #[cfg(feature = "advisor_profile")]
                let stamp = profile::Stamp::start();
                section.capture_section(
                    memory,
                    context,
                    observer,
                    roster,
                    (array.data, array.count),
                    unread != 0,
                );
                #[cfg(feature = "advisor_profile")]
                {
                    let elapsed = stamp.finish();
                    let sample = measurement_iter.next().unwrap();
                    sample[..2].copy_from_slice(&elapsed);
                    sample[2] = section.counts.read;
                    sample[3] = section.counts.reused;
                    sample[4] = section.counts.planned;
                    sample[5..10].copy_from_slice(&section.diagnostic);
                    sample[10] = section.characters as u64;
                }
                self.raw_bytes += section.raw_bytes;
                self.counts.planned += section.counts.planned;
                self.counts.reused += section.counts.reused;
                self.counts.read += section.counts.read;
                self.counts.roles += section.counts.roles;
                self.counts.verified += section.counts.verified;
                self.counts.mismatches += section.counts.mismatches;
                match section.result.outcome {
                    Outcome::Rows { len: n } => len += n,
                    Outcome::Truncated {
                        len: n,
                        unread_rows,
                    } => {
                        len += n;
                        unread = unread
                            .checked_add(unread_rows)
                            .ok_or("character missing row count overflow")?;
                    }
                    Outcome::Unavailable { reason } => return Err(reason),
                }
                self.result.bytes_len += section.result.bytes_len;
                self.result.required_bytes = self
                    .result
                    .required_bytes
                    .checked_add(section.result.required_bytes)
                    .ok_or("character byte count overflow")?;
            }
            self.result.outcome = if unread == 0 {
                Outcome::Rows { len }
            } else {
                Outcome::Truncated {
                    len,
                    unread_rows: unread,
                }
            };
            if before != observer.quiet() {
                return Err("advisor mutated during acquisition");
            }
            Ok(())
        })();
        #[cfg(feature = "advisor_profile")]
        profile::record(measurements);
        self.raw_bytes += counted.bytes.get();
        if let Err(reason) = result {
            self.result = ReadResult::rows(Outcome::Unavailable { reason });
            for section in &mut self.sections {
                section.valid = false;
                section.plan.invalidate();
            }
        }
    }
    pub fn decode(&self, captured: &mut Captured) {
        if let Outcome::Unavailable { .. } = self.result.outcome {
            captured.advisors.primary(self.result);
            return;
        }
        if let Err(reason) = captured.reserve_advisors(AdvisorDemand {
            rows: self.result.outcome.len(),
            bytes: self.result.bytes_len,
            overflowed: false,
        }) {
            captured
                .advisors
                .primary(ReadResult::rows(Outcome::Unavailable { reason }));
            return;
        }
        let mut row_start = 0;
        let mut byte_start = 0;
        for section in &self.sections {
            let len = section.result.outcome.len();
            let bytes = section.result.bytes_len;
            captured.advisors.bytes.first[byte_start..byte_start + bytes]
                .copy_from_slice(&section.current.bytes[..bytes]);
            for (target, source) in captured.advisors.rows.rows[row_start..row_start + len]
                .iter_mut()
                .zip(&section.current.rows[..len])
            {
                *target = *source;
                target.key_start += byte_start;
                target.idea_start += byte_start;
                target.slot_start += byte_start;
            }
            row_start += len;
            byte_start += bytes;
        }
        captured.advisors.primary(self.result);
    }
}

/// Hints follow their selecting pointer read. The prepared executor repeats
/// them only after that pointer's exact control comparison succeeds. This
/// overlaps independent character cache misses without changing read order.
struct Prefetched<'a, M> {
    memory: &'a M,
    geometry: (usize, u32),
}
impl<M: Memory> Memory for Prefetched<'_, M> {
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let ok = self.memory.copy(address, output);
        if ok
            && !output.is_empty()
            && output.len().is_multiple_of(8)
            && address.checked_sub(self.geometry.0).is_some_and(|offset| {
                offset.is_multiple_of(8)
                    && offset
                        .checked_add(output.len())
                        .is_some_and(|end| end <= self.geometry.1 as usize * 8)
            })
        {
            for pointer in output.chunks_exact(8) {
                let pointer = u64::from_le_bytes(pointer.try_into().unwrap()) as usize;
                for (offset, len) in [
                    (0, 16),
                    (ccharacter::COUNTRY, 4),
                    (ccharacter::ADVISOR_MAP_HEAD, 16),
                ] {
                    if let Some(address) = pointer.checked_add(offset) {
                        self.memory.prefetch(address, len);
                    }
                }
            }
        }
        ok
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        outputs: &mut [&mut [u8]; N],
    ) -> usize {
        let completed = self.memory.copy_scattered(addresses, outputs);
        // Recognize this reader's exact CCharacter::View field batch. Role
        // payload batches must not be interpreted as character map headers.
        if N == 7
            && completed == N
            && outputs[0].len() == 16
            && outputs[1].len() == 4
            && outputs[2..6].iter().all(|span| span.is_empty())
            && outputs[6].len() == 16
            && addresses[0].checked_add(ccharacter::COUNTRY) == Some(addresses[1])
            && addresses[0].checked_add(ccharacter::ADVISOR_MAP_HEAD) == Some(addresses[6])
        {
            let head = u64::from_le_bytes(outputs[6][..8].try_into().unwrap()) as usize;
            self.memory.prefetch(head, 26);
        }
        completed
    }
}

struct Counted<'a, M> {
    memory: &'a M,
    bytes: Cell<u64>,
    #[cfg(feature = "advisor_profile")]
    plan: Cell<[u64; 5]>,
}
impl<'a, M> Counted<'a, M> {
    fn new(memory: &'a M) -> Self {
        Self {
            memory,
            bytes: Cell::new(0),
            #[cfg(feature = "advisor_profile")]
            plan: Cell::new([0; 5]),
        }
    }
}
impl<M: Memory> Memory for Counted<'_, M> {
    fn prefetch(&self, address: usize, len: usize) {
        self.memory.prefetch(address, len);
    }
    fn execute_read_plan(
        &self,
        plan: &engine::read_plan::PreparedPlan,
        destination: &mut [u8],
    ) -> Option<engine::read_plan::Outcome> {
        use engine::read_plan::Outcome as PlanOutcome;
        #[cfg(feature = "advisor_profile")]
        let stamp = profile::Stamp::start();
        let result = self.memory.execute_read_plan(plan, destination);
        #[cfg(feature = "advisor_profile")]
        {
            let elapsed = stamp.finish();
            let (kind, read) = match result {
                Some(PlanOutcome::Complete) => (1, None),
                Some(PlanOutcome::ControlChanged { completed_reads }) => (
                    2,
                    completed_reads
                        .checked_sub(1)
                        .and_then(|i| plan.reads().get(i)),
                ),
                Some(PlanOutcome::ReadFailed { completed_reads }) => {
                    (3, plan.reads().get(completed_reads))
                }
                None => (4, None),
            };
            self.plan.set([
                elapsed[0],
                elapsed[1],
                kind,
                read.map_or(0, |r| r.address as u64),
                read.map_or(0, |r| r.len as u64),
            ]);
        }
        let bytes = match result {
            Some(PlanOutcome::Complete) => plan.destination_len(),
            Some(
                PlanOutcome::ControlChanged { completed_reads }
                | PlanOutcome::ReadFailed { completed_reads },
            ) => plan.reads()[..completed_reads]
                .iter()
                .map(|read| read.len)
                .sum(),
            None => 0,
        };
        self.bytes.set(self.bytes.get() + bytes as u64);
        result
    }
    fn copy(&self, address: usize, output: &mut [u8]) -> bool {
        let ok = self.memory.copy(address, output);
        if ok {
            self.bytes.set(self.bytes.get() + output.len() as u64);
        }
        ok
    }
    fn copy_scattered<const N: usize>(
        &self,
        addresses: [usize; N],
        outputs: &mut [&mut [u8]; N],
    ) -> usize {
        let copied = self.memory.copy_scattered(addresses, outputs);
        self.bytes.set(
            self.bytes.get()
                + outputs[..copied]
                    .iter()
                    .map(|output| output.len() as u64)
                    .sum::<u64>(),
        );
        copied
    }
    fn copy_scattered_prefetched<const N: usize>(
        &self,
        addresses: [usize; N],
        outputs: &mut [&mut [u8]; N],
    ) -> usize {
        let copied = self.memory.copy_scattered_prefetched(addresses, outputs);
        self.bytes.set(
            self.bytes.get()
                + outputs[..copied]
                    .iter()
                    .map(|output| output.len() as u64)
                    .sum::<u64>(),
        );
        copied
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        characters_test_kit as kit,
        tests::{ADDRESS, CHARACTERS, IMAGE, literal, pointer, put},
    };
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Observer {
        epoch: u64,
        clock: u64,
        changed: BTreeMap<usize, u64>,
        verify: bool,
        enabled: bool,
    }
    impl Observer {
        fn new() -> Self {
            Self {
                enabled: true,
                ..Default::default()
            }
        }
        fn change(&mut self, character: usize) {
            self.clock += 1;
            self.changed.insert(
                ADDRESS + character + ccharacter::ADVISOR_MAP_HEAD,
                self.clock,
            );
        }
    }
    impl Changes for Observer {
        fn quiet(&self) -> Option<(u64, u64)> {
            self.enabled.then_some((self.epoch, self.clock))
        }
        fn revision(&self, map: usize) -> Option<(u64, u64)> {
            self.enabled
                .then_some((self.epoch, *self.changed.get(&map).unwrap_or(&0)))
        }
        fn verify(&self) -> bool {
            self.verify
        }
    }
    fn context() -> CharactersCopyContext {
        CharactersCopyContext {
            base: IMAGE,
            game_state: ADDRESS,
            active: PART_MASKS[2],
            trait_rows: [0; 2],
            trait_bytes: [0; 2],
        }
    }
    fn assert_fresh(cache: &Section, bytes: &[u8]) {
        let mut captured = Captured::with_capacities(&Capacities::default());
        let direct = walk_family(
            &kit::memory(bytes),
            IMAGE,
            ADDRESS,
            captured.family().select(PART_MASKS[2]),
            None,
        )[2];
        assert_eq!(cache.result.outcome, direct.outcome);
        assert_eq!(cache.result.bytes_len, direct.bytes_len);
        assert_eq!(cache.result.required_bytes, direct.required_bytes);
        assert_eq!(
            cache.current.rows[..direct.outcome.len()],
            captured.advisors.rows.rows[..direct.outcome.len()]
        );
        assert_eq!(
            cache.current.bytes[..direct.bytes_len],
            captured.advisors.bytes.first[..direct.bytes_len]
        );
    }

    fn assert_outer_fresh(cache: &Cache, bytes: &[u8]) {
        let mut captured = Captured::with_capacities(&Capacities::default());
        cache.decode(&mut captured);
        let mut direct = Captured::with_capacities(&Capacities::default());
        let expected = walk_family(
            &kit::memory(bytes),
            IMAGE,
            ADDRESS,
            direct.family().select(PART_MASKS[2]),
            None,
        )[2];
        assert_eq!(cache.result.outcome, expected.outcome);
        assert_eq!(cache.result.required_bytes, expected.required_bytes);
        let len = expected.outcome.len();
        assert_eq!(
            captured.advisors.rows.rows[..len],
            direct.advisors.rows.rows[..len]
        );
        assert_eq!(
            captured.advisors.bytes.first[..expected.bytes_len],
            direct.advisors.bytes.first[..expected.bytes_len]
        );
    }
    #[test]
    fn first_capture_prepares_controls_without_a_second_roster_discovery() {
        let bytes = super::super::tests::section_fixture();
        let mut observer = Observer::new();
        let mut cache = Cache::new();
        cache.capture(&kit::memory(&bytes), context(), &observer);
        assert_eq!(cache.counts.read, 2);
        cache.prepare();
        cache.capture(&kit::memory(&bytes), context(), &observer);
        assert_eq!(cache.counts.planned, 2);
        assert_outer_fresh(&cache, &bytes);
        observer.enabled = false;
        cache.capture(&kit::memory(&bytes), context(), &observer);
        cache.prepare();
        observer.enabled = true;
        cache.capture(&kit::memory(&bytes), context(), &observer);
        assert_eq!(cache.counts.planned, 0);
        assert_outer_fresh(&cache, &bytes);
    }
    #[test]
    fn unregistered_maps_cannot_use_a_control_only_certificate() {
        struct Unwatched;
        impl Changes for Unwatched {
            fn quiet(&self) -> Option<(u64, u64)> {
                Some((0, 0))
            }
            fn revision(&self, _map: usize) -> Option<(u64, u64)> {
                None
            }
            fn verify(&self) -> bool {
                false
            }
        }
        let mut bytes = super::super::tests::section_fixture();
        let mut cache = Cache::new();
        for idea in [b"first".as_slice(), b"second", b"third"] {
            literal(&mut bytes, kit::ADV_ROLE + cadvisor::IDEA_TOKEN, idea);
            cache.capture(&kit::memory(&bytes), context(), &Unwatched);
            assert_eq!(cache.counts.planned, 0);
            assert_eq!(cache.counts.read, 2);
            assert_outer_fresh(&cache, &bytes);
            cache.prepare();
        }
    }
    fn warm(cache: &mut Cache, bytes: &[u8], observer: &Observer) {
        for _ in 0..3 {
            cache.capture(&kit::memory(bytes), context(), observer);
            assert_outer_fresh(cache, bytes);
            cache.prepare();
        }
        assert_eq!(cache.counts.planned, 2);
    }
    #[test]
    fn independent_plans_refresh_country_mutations_aliases_and_roster_growth() {
        use super::super::tests::{MANAGER, array};
        let mut bytes = super::super::tests::section_fixture();
        let mut observer = Observer::new();
        let mut cache = Cache::new();
        warm(&mut cache, &bytes, &observer);
        array(
            &mut bytes,
            MANAGER + ccharacter_manager::DYNAMIC_CHARACTERS,
            0x8800,
            2,
        );
        pointer(&mut bytes, 0x8808, 0x2300);
        cache.capture(&kit::memory(&bytes), context(), &observer);
        assert_eq!(cache.sections[0].counts.planned, 1);
        assert_eq!(cache.sections[1].counts.planned, 0);
        assert_outer_fresh(&cache, &bytes);
        warm(&mut cache, &bytes, &observer);
        put(
            &mut bytes,
            CHARACTERS[0] + ccharacter::COUNTRY,
            99_u32.to_le_bytes(),
        );
        cache.capture(&kit::memory(&bytes), context(), &observer);
        assert_eq!(cache.sections[0].counts.planned, 1);
        assert_eq!(cache.sections[1].counts.planned, 1);
        assert_outer_fresh(&cache, &bytes);
        warm(&mut cache, &bytes, &observer);
        literal(&mut bytes, kit::ADV_ROLE + cadvisor::IDEA_TOKEN, b"changed");
        observer.change(CHARACTERS[0]);
        cache.capture(&kit::memory(&bytes), context(), &observer);
        assert_eq!(cache.sections[0].counts.read, 1);
        assert_eq!(cache.sections[1].counts.planned, 1);
        assert_outer_fresh(&cache, &bytes);
        pointer(&mut bytes, 0x8800, CHARACTERS[0]);
        cache.capture(&kit::memory(&bytes), context(), &observer);
        assert_outer_fresh(&cache, &bytes);
        warm(&mut cache, &bytes, &observer);
        observer.epoch += 1;
        cache.capture(&kit::memory(&bytes), context(), &observer);
        assert_eq!(cache.counts.reused, 0);
        assert_outer_fresh(&cache, &bytes);
    }
    #[test]
    fn planned_control_failures_keep_first_fault_and_recover_with_fresh_data() {
        use super::super::tests::WaveMemory;
        let bytes = super::super::tests::section_fixture();
        let observer = Observer::new();
        let mut cache = Cache::new();
        warm(&mut cache, &bytes, &observer);
        let trace = WaveMemory::new(&bytes, None);
        cache.capture(&trace, context(), &observer);
        assert_eq!(cache.counts.planned, 2);
        let faults: std::collections::BTreeSet<_> = trace
            .reads
            .borrow()
            .iter()
            .flat_map(|&(address, len)| [address, address + len - 1])
            .collect();
        for fault in faults {
            warm(&mut cache, &bytes, &observer);
            let source = WaveMemory::new(&bytes, Some(fault));
            cache.capture(&source, context(), &observer);
            assert!(
                matches!(cache.result.outcome, Outcome::Unavailable { .. }),
                "{fault:x}"
            );
            assert_eq!(
                source
                    .reads
                    .borrow()
                    .iter()
                    .filter(|&&(a, n)| (a..a + n).contains(&fault))
                    .count(),
                1,
                "{fault:x}"
            );
            assert!(cache.sections.iter().all(|section| !section.valid));
        }
    }

    #[test]
    fn planned_country_prefix_survives_later_control_change_and_fault() {
        use super::super::tests::WaveMemory;
        let mut original = kit::mapped_fixture();
        add_second_role(&mut original);
        let header = CHARACTERS[1] + ccharacter::ADVISOR_MAP_HEAD;
        let sentinel =
            u64::from_le_bytes(original[header..header + 8].try_into().unwrap()) as usize;
        let country = ADDRESS + CHARACTERS[0] + ccharacter::COUNTRY;
        let observer = Observer::new();
        for fault in [false, true] {
            let mut cache = Cache::new();
            warm(&mut cache, &original, &observer);
            let baseline = WaveMemory::new(&original, None);
            cache.capture(&baseline, context(), &observer);
            assert_eq!(cache.sections[0].counts.planned, 1);
            let expected_reads = baseline.reads.borrow().clone();
            let country_index = expected_reads
                .iter()
                .position(|&read| read == (country, 4))
                .unwrap();
            let stop_index = expected_reads
                .iter()
                .position(|&read| read == (sentinel, 26))
                .unwrap();
            assert!(country_index < stop_index);

            let mut bytes = original.clone();
            put(
                &mut bytes,
                CHARACTERS[0] + ccharacter::COUNTRY,
                99_u32.to_le_bytes(),
            );
            // A changed, readable color byte stops the equality control without
            // invalidating the map. The alternate case partially copies and fails.
            bytes[sentinel - ADDRESS + engine::msvc_tree_node::COLOR] ^= 1;
            let source = WaveMemory::new(&bytes, fault.then_some(sentinel));
            cache.capture(&source, context(), &observer);
            assert_eq!(cache.sections[0].counts.planned, 0);
            let reads = source.reads.borrow();
            assert_eq!(reads[..=stop_index], expected_reads[..=stop_index]);
            for read in [(country, 4), (sentinel, 26)] {
                assert_eq!(reads.iter().filter(|&&seen| seen == read).count(), 1);
            }
            assert_eq!(cache.sections[0].current.rows[0].country, 99);
            if fault {
                assert_eq!(
                    cache.result.outcome,
                    Outcome::Unavailable {
                        reason: "character role map sentinel unreadable",
                    }
                );
                assert_eq!(reads.as_slice(), &expected_reads[..=stop_index]);
                assert!(cache.sections.iter().all(|section| !section.valid));
                let mut captured = Captured::with_capacities(&Capacities::default());
                cache.decode(&mut captured);
                assert_eq!(captured.advisors.rows.outcome, cache.result.outcome);
            } else {
                assert_eq!(cache.sections[0].counts.read, 1);
                assert_eq!(cache.sections[0].counts.reused, 2);
                assert_outer_fresh(&cache, &bytes);
            }
        }
    }

    #[test]
    fn planned_duplicate_characters_keep_each_country_observation() {
        use super::super::tests::{ROSTER, WaveMemory};
        struct Countries<'a> {
            bytes: &'a [u8],
            values: [u32; 3],
            next: Cell<usize>,
            reads: std::cell::RefCell<Vec<(usize, usize)>>,
        }
        impl<'a> Countries<'a> {
            fn new(bytes: &'a [u8], values: [u32; 3]) -> Self {
                Self {
                    bytes,
                    values,
                    next: Cell::new(0),
                    reads: Default::default(),
                }
            }
        }
        impl Memory for Countries<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                self.reads.borrow_mut().push((address, output.len()));
                if address == ADDRESS + CHARACTERS[0] + ccharacter::COUNTRY && output.len() == 4 {
                    let index = self.next.get();
                    let value = self.values.get(index).expect("country occurrence retried");
                    self.next.set(index + 1);
                    output.copy_from_slice(&value.to_le_bytes());
                    true
                } else {
                    kit::memory(self.bytes).copy(address, output)
                }
            }
        }
        let mut bytes = kit::mapped_fixture();
        pointer(&mut bytes, ROSTER + 8, CHARACTERS[0]);
        pointer(&mut bytes, ROSTER + 16, CHARACTERS[0]);
        let observer = Observer::new();
        let mut cache = Cache::new();
        warm(&mut cache, &bytes, &observer);
        let baseline = WaveMemory::new(&bytes, None);
        cache.capture(&baseline, context(), &observer);
        let expected_reads = baseline.reads.borrow().clone();
        let country_read = (ADDRESS + CHARACTERS[0] + ccharacter::COUNTRY, 4);
        assert_eq!(
            expected_reads
                .iter()
                .filter(|&&read| read == country_read)
                .count(),
            3
        );

        for countries in [[11, 22, 33], [44, 55, 66]] {
            let source = Countries::new(&bytes, countries);
            cache.capture(&source, context(), &observer);
            assert_eq!(cache.sections[0].counts.planned, 1);
            assert_eq!(cache.counts.read, 0);
            assert_eq!(source.next.get(), 3);
            assert_eq!(*source.reads.borrow(), expected_reads);

            // A separate observation sequence gives the ordinary reader the
            // same three distinct values; an address-keyed oracle would hide bugs.
            let direct_source = Countries::new(&bytes, countries);
            let mut direct = Captured::with_capacities(&Capacities::default());
            let expected = walk_family(
                &direct_source,
                IMAGE,
                ADDRESS,
                direct.family().select(PART_MASKS[2]),
                None,
            )[2];
            assert_eq!(direct_source.next.get(), 3);
            assert_eq!(expected.outcome, Outcome::Rows { len: 3 });
            assert_eq!(cache.result.outcome, expected.outcome);
            assert_eq!(cache.result.bytes_len, expected.bytes_len);
            assert_eq!(cache.result.required_bytes, expected.required_bytes);
            let mut captured = Captured::with_capacities(&Capacities::default());
            cache.decode(&mut captured);
            assert_eq!(
                captured.advisors.rows.rows[..3],
                direct.advisors.rows.rows[..3]
            );
            assert_eq!(
                [0, 1, 2].map(|index| captured.advisors.rows.rows[index].country),
                countries
            );
            assert_eq!(
                captured.advisors.bytes.first[..expected.bytes_len],
                direct.advisors.bytes.first[..expected.bytes_len]
            );
            cache.prepare();
        }
    }

    #[test]
    fn earlier_truncation_suppresses_later_rows_and_retains_missing_demand() {
        let mut bytes = super::super::tests::section_fixture();
        add_second_role(&mut bytes);
        let observer = Observer::new();
        let mut cache = Cache::new();
        cache.sections[0] = Section::with_capacity(0, 0, 8);
        cache.capture(&kit::memory(&bytes), context(), &observer);
        assert!(matches!(
            cache.result.outcome,
            Outcome::Truncated {
                len: 0,
                unread_rows: 2
            }
        ));
        assert_eq!(cache.sections[1].result.outcome.len(), 0);
        for _ in 0..3 {
            cache.grow();
            cache.capture(&kit::memory(&bytes), context(), &observer);
        }
        assert_outer_fresh(&cache, &bytes);
        warm(&mut cache, &bytes, &observer);
    }

    fn add_second_role(bytes: &mut [u8]) {
        let character = CHARACTERS[1];
        let offset = character + ccharacter::ADVISOR_MAP_HEAD;
        let head =
            u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap()) as usize - ADDRESS;
        let node = 0x8500;
        let role = 0x8580;
        bytes.copy_within(kit::ADV_NODE..kit::ADV_NODE + 0x50, node);
        bytes.copy_within(kit::ADV_ROLE..kit::ADV_ROLE + 0xa0, role);
        for link in [0, 8, 16] {
            pointer(bytes, head + link, node);
            pointer(bytes, node + link, head);
        }
        put(bytes, offset + 8, 1_u64.to_le_bytes());
        pointer(bytes, node + character_advisor_node::ADVISOR, role);
        pointer(bytes, role + cadvisor::CHARACTER, character);
    }

    #[test]
    fn role_addition_removal_and_arena_relocation_keep_other_characters_reusable() {
        let mut bytes = kit::mapped_fixture();
        let mut cache = Section::with_capacity(8, 512, 8);
        let mut observer = Observer::new();
        cache.capture(&kit::memory(&bytes), context(), &observer);
        add_second_role(&mut bytes);
        observer.change(CHARACTERS[1]);
        cache.capture(&kit::memory(&bytes), context(), &observer);
        assert_fresh(&cache, &bytes);
        assert_eq!((cache.counts.read, cache.counts.reused), (1, 2));
        literal(
            &mut bytes,
            kit::ADV_ROLE + cadvisor::IDEA_TOKEN,
            b"longer_first",
        );
        observer.change(CHARACTERS[0]);
        cache.capture(&kit::memory(&bytes), context(), &observer);
        assert_fresh(&cache, &bytes);
        assert_eq!(
            (cache.counts.read, cache.counts.reused, cache.counts.roles),
            (1, 2, 1)
        );
        put(
            &mut bytes,
            CHARACTERS[0] + ccharacter::ADVISOR_MAP_HEAD + 8,
            0_u64.to_le_bytes(),
        );
        for link in [0, 8, 16] {
            pointer(&mut bytes, 0x2430 + link, 0x2430);
        }
        observer.change(CHARACTERS[0]);
        cache.capture(&kit::memory(&bytes), context(), &observer);
        assert_fresh(&cache, &bytes);
        assert_eq!(
            (cache.counts.read, cache.counts.reused, cache.counts.roles),
            (1, 2, 1)
        );
    }
    #[test]
    fn unchanged_roles_skip_payload_reads_but_country_stays_current() {
        let mut bytes = kit::mapped_fixture();
        let mut cache = Section::with_capacity(8, 512, 8);
        let observer = Observer::new();
        cache.capture(&kit::memory(&bytes), context(), &observer);
        assert_fresh(&cache, &bytes);
        let cold = cache.raw_bytes;
        put(
            &mut bytes,
            CHARACTERS[0] + ccharacter::COUNTRY,
            99_u32.to_le_bytes(),
        );
        cache.capture(&kit::memory(&bytes), context(), &observer);
        assert_fresh(&cache, &bytes);
        assert_eq!(
            (cache.counts.reused, cache.counts.read, cache.counts.roles),
            (3, 0, 1)
        );
        assert!(cache.raw_bytes < cold);
        assert_eq!(cache.current.rows[0].country, 99);
        // Decode uses only owned bytes, even after all source storage disappears.
        bytes.fill(0);
        let mut output = Captured::with_capacities(&Capacities::default());
        cache.decode(&mut output);
        assert_eq!(output.advisors.rows.rows[0].country, 99);
    }
    #[test]
    fn same_count_replacement_and_larger_strings_refresh_only_the_affected_character() {
        let mut bytes = kit::mapped_fixture();
        let mut cache = Section::with_capacity(8, 512, 8);
        let mut observer = Observer::new();
        cache.capture(&kit::memory(&bytes), context(), &observer);
        bytes.copy_within(kit::ADV_ROLE..kit::ADV_ROLE + 0xa0, 0x8600);
        pointer(
            &mut bytes,
            kit::ADV_NODE + character_advisor_node::ADVISOR,
            0x8600,
        );
        literal(&mut bytes, 0x8600 + cadvisor::IDEA_TOKEN, b"different_idea");
        observer.change(CHARACTERS[0]);
        cache.capture(&kit::memory(&bytes), context(), &observer);
        assert_fresh(&cache, &bytes);
        assert_eq!((cache.counts.read, cache.counts.reused), (1, 2));
        // Epoch invalidation forces all entries fresh, including empty maps.
        observer.epoch += 1;
        cache.capture(&kit::memory(&bytes), context(), &observer);
        assert_fresh(&cache, &bytes);
        assert_eq!((cache.counts.read, cache.counts.reused), (3, 0));
    }
    #[test]
    fn verification_detects_an_unobserved_in_place_write_and_returns_fresh_data() {
        let mut bytes = kit::mapped_fixture();
        let mut cache = Section::with_capacity(8, 512, 8);
        let mut observer = Observer::new();
        observer.verify = true;
        cache.capture(&kit::memory(&bytes), context(), &observer);
        literal(
            &mut bytes,
            kit::ADV_ROLE + cadvisor::IDEA_TOKEN,
            b"unobserved",
        );
        cache.capture(&kit::memory(&bytes), context(), &observer);
        assert_fresh(&cache, &bytes);
        assert_eq!(cache.counts.verified, 3);
        assert_eq!(cache.counts.mismatches, 1);
        assert!(!cache.valid);
        cache.capture(&kit::memory(&bytes), context(), &observer);
        assert_eq!(cache.counts.verified, 0);
    }
    #[test]
    fn truncation_and_unwatched_entries_stay_fresh_and_grow_off_capture() {
        let bytes = kit::mapped_fixture();
        let mut cache = Section::with_capacity(0, 0, 1);
        let mut observer = Observer::new();
        cache.capture(&kit::memory(&bytes), context(), &observer);
        assert!(matches!(cache.result.outcome, Outcome::Truncated { .. }));
        assert!(!cache.valid);
        for _ in 0..3 {
            cache.grow();
            cache.capture(&kit::memory(&bytes), context(), &observer);
        }
        assert_fresh(&cache, &bytes);
        observer.enabled = false;
        cache.capture(&kit::memory(&bytes), context(), &observer);
        assert_fresh(&cache, &bytes);
        assert_eq!(cache.counts.reused, 0);
    }
    #[test]
    fn fresh_control_faults_and_dirty_role_faults_never_reuse_stale_success() {
        let bytes = kit::mapped_fixture();
        let mut observer = Observer::new();
        struct Fault<'a> {
            bytes: &'a [u8],
            address: usize,
            attempts: Cell<usize>,
        }
        impl Memory for Fault<'_> {
            fn copy(&self, address: usize, output: &mut [u8]) -> bool {
                if address <= self.address && self.address < address + output.len() {
                    self.attempts.set(self.attempts.get() + 1);
                    return false;
                }
                kit::memory(self.bytes).copy(address, output)
            }
        }
        for address in [
            ADDRESS + CHARACTERS[0] + ccharacter::COUNTRY,
            ADDRESS + CHARACTERS[0] + ccharacter::ADVISOR_MAP_HEAD,
            ADDRESS + 0x2430,
            ADDRESS + kit::ADV_ROLE + cadvisor::IDEA_TOKEN,
        ] {
            let mut cache = Section::with_capacity(8, 512, 8);
            cache.capture(&kit::memory(&bytes), context(), &observer);
            observer.change(CHARACTERS[0]);
            let fault = Fault {
                bytes: &bytes,
                address,
                attempts: Cell::new(0),
            };
            cache.capture(&fault, context(), &observer);
            assert!(matches!(cache.result.outcome, Outcome::Unavailable { .. }));
            assert_eq!(fault.attempts.get(), 1);
            assert!(!cache.valid);
        }
    }
}
