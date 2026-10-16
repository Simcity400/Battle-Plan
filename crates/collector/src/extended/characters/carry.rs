//! Characters rows carried between hours by the roster job
//! ([`crate::character_changes`]).
//!
//! A carried character's row is published from the held read without copying its name-token span
//! (from `ccharacter::NAME_TOKEN_ID` up to `ccharacter::NAME`, which holds the template pointer),
//! its two name strings and their heap payloads, its gender or its template. Its identity, country
//! and roster position are still read every hour, so the row's id, country, roster and ordinal are
//! this hour's. A character is carried only when its pointer and id equal the held read's and no
//! hooked writer touched it since that read registered it (same watch epoch and sequence). Every
//! writer of those fields is a hooked site: the character name setter and Character identity repair
//! ([`engine::character_role_mutations`]), the character parser, merge and destructor (advisor
//! family); a character the character definition builder writes is a new object, and an address is
//! reused only after the hooked destructor ran. A template's identity is a database value for its
//! pointer ([`super::TemplateCache`]).
//!
//! The set of carried characters keys the roster job's prepared read program
//! (`generation`): the program omits exactly their name reads, and a changed
//! set rediscovers the roster. Each hour first re-certifies the set; a
//! character a writer touched leaves it, and characters certified since are
//! promoted at a later rediscovery, at once when the set is empty. The decode
//! worker rebuilds the held rows only from a rediscovering pass, whose
//! visits pair each roster occurrence with its certificate.
//!
//! The re-certification is looked up entry by entry only when the character
//! domain's quiet token ([`Changes::quiet`]) moved since the last lookup: an
//! unchanged token means no observed writer ran, so every held certificate
//! certifies as it did. A full pass's roster walk still reads every
//! occurrence's identity, country and roster position and the unit leaders
//! it reaches. A quiet roster hour ([`super::quiet`]) walks no roster: when
//! every occurrence of the held census is certified it carries all of them
//! ([`Carry::certifies`], [`Carry::carry_certified`]), each row taking this
//! hour's country from the role census.
//!
//! Every hour first reads the carried characters of its sampled-verification
//! slice ([`crate::sampled_verify`], keyed by character id) afresh from live
//! memory and compares each with its held row ([`Carry::sample`]); a
//! difference poisons reuse and empties the carried set, so that hour's
//! pass reads and publishes every name afresh. Verify mode carries nothing,
//! and every hour compares each certified character's fresh row with its
//! held row, poisoning reuse on a difference.
use super::*;
use crate::{
    character_changes::{Changes, Counts, Table},
    sampled_verify::Check,
};
use std::ops::Range;

/// An `order` position without a held entry.
const UNHELD: usize = usize::MAX;

#[derive(Clone, Debug)]
struct Entry {
    pointer: usize,
    id: u32,
    certificate: (u64, u64),
    /// Text offsets relative to `bytes.start`.
    row: ccharacter::Row,
    bytes: Range<usize>,
}

/// One roster occurrence of a rediscovering pass.
#[derive(Clone, Copy, Debug)]
pub(super) struct Visit {
    pointer: usize,
    certificate: Option<(u64, u64)>,
}

pub(crate) struct Carry {
    entries: Vec<Entry>,
    bytes: Vec<u8>,
    /// Roster order of the latest rediscovering pass.
    order: Vec<usize>,
    /// Per `order` position, the index of its held entry; [`UNHELD`] for
    /// an occurrence without one. A quiet roster hour visits the census in
    /// roster order, so its lookups follow this instead of searching.
    order_entries: Vec<usize>,
    /// Pointers whose rows this hour's acquisition carries, sorted.
    carried: Vec<usize>,
    /// Keys the prepared program with the carried set.
    pub(super) generation: u64,
    /// Names the held entries and text: a fresh value, unique across jobs,
    /// whenever they change ([`Self::hold`], [`Self::forget`]).
    pub(super) version: u64,
    /// [`Self::census_demand`] of the census generation and version it was
    /// summed for.
    demand: Option<((u64, u64), (usize, usize))>,
    /// Per entry: certified at this hour's start.
    certified: Vec<bool>,
    /// The domain's quiet token taken before `certified` was last looked up
    /// entry by entry. While it repeats no observed writer ran, so every
    /// sequence, and `certified`, is unchanged.
    certified_quiet: Option<(u64, u64)>,
    promote: bool,
    visits: Vec<Visit>,
    visits_failed: bool,
    /// The payload came from a rediscovering pass.
    producer: bool,
    /// This payload's Characters rows follow the carried set.
    pub(super) active: bool,
    changes: &'static dyn Changes,
    /// Verify mode: carry nothing and compare every certified row.
    verify: bool,
    /// Tests: the hours' sampled comparison; otherwise the hour's
    /// ([`crate::sampled_verify::current_burned_in`]).
    check: Option<Check>,
    /// Text of a sampled character's fresh read.
    scratch: Vec<u8>,
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
        Self::with(&crate::character_changes::Observer, None)
    }
}

impl Carry {
    /// `check` overrides the hour's sampled comparison; [`Check::Every`] is
    /// verify mode.
    pub(super) fn with(changes: &'static dyn Changes, check: Option<Check>) -> Self {
        let verify = check.map_or(crate::character_changes::VERIFY, |check| {
            check == Check::Every
        });
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
            verify,
            check,
            scratch: Vec::new(),
            counts: Counts::default(),
        }
    }

    #[cfg(test)]
    pub(super) fn counts(&self) -> Counts {
        self.counts
    }

    #[cfg(test)]
    pub(super) fn set_check(&mut self, check: Check) {
        self.check = Some(check);
    }

    pub(super) fn allocated_bytes(&self) -> u64 {
        crate::raw_memory::vector_bytes(&self.entries)
            + crate::raw_memory::vector_bytes(&self.bytes)
            + crate::raw_memory::vector_bytes(&self.order)
            + crate::raw_memory::vector_bytes(&self.order_entries)
            + crate::raw_memory::vector_bytes(&self.carried)
            + crate::raw_memory::vector_bytes(&self.certified)
            + crate::raw_memory::vector_bytes(&self.visits)
            + crate::raw_memory::vector_bytes(&self.scratch)
    }

    fn position(&self, pointer: usize) -> Option<usize> {
        self.entries
            .binary_search_by_key(&pointer, |entry| entry.pointer)
            .ok()
    }

    /// [`Self::position`] of the occurrence at roster `index` (historical
    /// array first): read from the held order when it names `pointer` there.
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

    /// Copy side, before the program key: certify every held row against
    /// this hour's sequences and settle the carried set. `active` is whether
    /// this pass reads Characters as the roster job's hourly pass.
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
        // Taken before any lookup it stands for.
        let token = changes.quiet();
        let quiet = token.is_some()
            && token == self.certified_quiet
            && self.certified.len() == self.entries.len();
        if !quiet {
            self.certified.clear();
            self.certified_quiet = None;
        }
        let mut next = Vec::new();
        let prepared = engine::allocation::advisory(|| {
            engine::allocation::reserve_exact(&mut self.certified, self.entries.len())?;
            engine::allocation::reserve_exact(&mut next, self.entries.len())?;
            std::io::Result::Ok(())
        });
        if prepared.is_err() {
            self.forget();
            return;
        }
        if quiet {
            self.counts.quiet = true;
        } else {
            // Registration of every held pointer precedes the reads it
            // certifies.
            for entry in &self.entries {
                self.certified
                    .push(changes.sequence(entry.pointer) == Some(entry.certificate));
            }
            self.certified_quiet = token;
        }
        if !self.verify {
            let promote = std::mem::take(&mut self.promote) || self.carried.is_empty();
            // The carried set is a sorted subset of the entries' pointers,
            // so one merge finds each entry's membership.
            let mut carried = self.carried.iter().peekable();
            for (entry, &certified) in self.entries.iter().zip(&self.certified) {
                let held = carried.next_if_eq(&&entry.pointer).is_some();
                if certified && (promote || held) {
                    next.push(entry.pointer);
                }
            }
        }
        if next != self.carried {
            self.carried = next;
            self.generation = self.generation.wrapping_add(1);
        }
    }

    /// Copy side, after [`Self::begin_hour`] and before the program key: read
    /// the carried characters of this hour's sampled slice afresh from live
    /// `memory` and compare each with its held row. A difference poisons
    /// reuse and empties the carried set, so this hour's pass reads every
    /// name afresh. A fresh read that fails compares nothing.
    pub(super) fn sample(&mut self, memory: &impl engine::Memory, base: usize) {
        let check = self
            .check
            .unwrap_or_else(|| crate::sampled_verify::current_burned_in(false));
        if !self.active || self.verify || !check.any() || self.carried.is_empty() {
            return;
        }
        let (mut verified, mut mismatches) = (0, 0);
        for entry in &self.entries {
            if !check.covers(u64::from(entry.id))
                || self.carried.binary_search(&entry.pointer).is_err()
            {
                continue;
            }
            let held = &self.bytes[entry.bytes.clone()];
            // A fresh text longer than the held one does not fit, and differs.
            if self.scratch.len() < held.len()
                && engine::allocation::advisory(|| {
                    engine::allocation::resize(&mut self.scratch, held.len(), 0)
                })
                .is_err()
            {
                continue;
            }
            let mut row = [ccharacter::Row::default()];
            let fresh = ccharacter::walk_entry(
                memory,
                base,
                ccharacter_manager::Entry {
                    pointer: entry.pointer,
                    id: entry.id,
                    country: entry.row.country,
                    roster: entry.row.roster,
                    ordinal: entry.row.ordinal,
                },
                |_| None,
                &mut row,
                &mut self.scratch[..held.len()],
            );
            let same = match fresh.outcome {
                Outcome::Rows { len: 1 } => same(&row[0], &self.scratch, &entry.row, held),
                Outcome::Truncated { .. } => false,
                _ => continue,
            };
            verified += 1;
            mismatches += u64::from(!same);
        }
        self.counts.verified += verified;
        self.counts.mismatches += mismatches;
        if mismatches != 0 {
            self.changes.mismatch();
            self.drop_carried();
            // Compared once: the pass's fresh rows are not compared again.
            self.certified.fill(false);
            self.certified_quiet = None;
        }
    }

    /// Quiet roster hour ([`super::quiet`]), after [`Self::begin_hour`] and
    /// [`Self::sample`]: whether every occurrence of the held census names a
    /// held character with the same pointer and id whose certificate holds
    /// this hour. Verify mode carries nothing, so it certifies no hour.
    pub(super) fn certifies(&self, occurrences: &[ccharacter_manager::PortraitIdentity]) -> bool {
        self.active
            && !self.verify
            && self.certified.len() == self.entries.len()
            && occurrences.iter().enumerate().all(|(at, occurrence)| {
                self.position_at(at, occurrence.pointer)
                    .is_some_and(|index| {
                        self.certified[index] && self.entries[index].id == occurrence.character
                    })
            })
    }

    /// Quiet roster hour, after [`Self::certifies`]: carry every certified
    /// character this pass, keying the next program with the new set. False
    /// when the set cannot be stored (worker storage is advisory).
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
        if engine::allocation::advisory(|| engine::allocation::reserve_exact(&mut next, count))
            .is_err()
        {
            return false;
        }
        // Entries are sorted by pointer, so the set is too.
        next.extend(certified(&self.entries, &self.certified));
        self.carried = next;
        self.generation = self.generation.wrapping_add(1);
        true
    }

    /// Whether this hour's acquisition omits `pointer`'s name reads.
    pub(super) fn carries(&self, pointer: usize) -> bool {
        self.active && self.carried.binary_search(&pointer).is_ok()
    }

    /// The held row of a carried occurrence, with its text; `None` when the
    /// occurrence is not carried or names another character.
    pub(super) fn carried_row(&self, pointer: usize, id: u32) -> Option<(ccharacter::Row, &[u8])> {
        if !self.carries(pointer) {
            return None;
        }
        let entry = &self.entries[self.position(pointer)?];
        (entry.id == id).then(|| (entry.row, &self.bytes[entry.bytes.clone()]))
    }

    /// Text bytes a carried occurrence appends.
    pub(super) fn carried_bytes(&self, pointer: usize) -> usize {
        self.text_len(self.position(pointer))
    }

    /// The rows and text bytes a quiet hour that carries every occurrence of
    /// `census` publishes. A function of the census (its nonzero generation)
    /// and the held reads (the version), so it is summed once per pair.
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

    /// A carried pointer turned out to name another character: read it this
    /// pass and rediscover the program.
    pub(super) fn uncarry(&mut self, pointer: usize) {
        if let Ok(index) = self.carried.binary_search(&pointer) {
            self.carried.remove(index);
            self.generation = self.generation.wrapping_add(1);
        }
    }

    /// Copy side, rediscovering pass: one roster occurrence, registering it.
    pub(super) fn visit(&mut self, pointer: usize, carried: bool) {
        let certificate = if carried {
            self.position(pointer)
                .map(|index| self.entries[index].certificate)
        } else {
            self.changes.sequence(pointer)
        };
        if engine::allocation::advisory(|| engine::allocation::reserve(&mut self.visits, 1))
            .is_err()
        {
            self.visits_failed = true;
            return;
        }
        self.visits.push(Visit {
            pointer,
            certificate,
        });
    }

    /// Copy side: the rediscovering pass visited the whole roster.
    pub(super) fn finish_pass(&mut self, complete: bool) {
        self.producer = self.active && complete && !self.visits_failed;
    }

    /// Decode worker, after the Characters rows of this payload: verify
    /// certified rows read afresh, then hold a rediscovering pass's rows.
    /// `quiet`: the rows came from a quiet roster hour, which carries every
    /// occurrence ([`Self::carry_certified`]), so none was read afresh.
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
            crate::character_changes::record(Table::Characters, &self.counts);
            return;
        };
        if quiet {
            self.counts.reused = rows.len() as u64;
            self.counts.read = 0;
            crate::character_changes::record(Table::Characters, &self.counts);
            return;
        }
        let carried = rows
            .iter()
            .zip(self.order_for(rows.len()))
            .filter(|&(_, pointer)| self.carried.binary_search(&pointer).is_ok())
            .count();
        self.counts.reused = carried as u64;
        self.counts.read = (rows.len() - carried) as u64;
        // A row read afresh while its held row stays certified must equal it.
        let (mut verified, mut mismatches) = (0, 0);
        for (row, pointer) in rows.iter().zip(self.order_for(rows.len())) {
            if self.carried.binary_search(&pointer).is_ok() {
                continue;
            }
            let Some(index) = self.position(pointer) else {
                continue;
            };
            let entry = &self.entries[index];
            if !self.certified.get(index).copied().unwrap_or(false) || entry.id != row.id {
                continue;
            }
            verified += 1;
            if !same(row, bytes, &entry.row, &self.bytes[entry.bytes.clone()]) {
                mismatches += 1;
            }
        }
        // After this hour's sampled comparison ([`Self::sample`]).
        self.counts.verified += verified;
        self.counts.mismatches += mismatches;
        if mismatches != 0 {
            self.changes.mismatch();
        }
        if self.producer {
            if self.visits.len() == rows.len() {
                self.hold(rows, bytes);
            } else {
                self.forget();
            }
        }
        crate::character_changes::record(Table::Characters, &self.counts);
    }

    /// The pointers of `len` rows: this pass's visits, or the latest
    /// rediscovering pass's order, which a prepared pass repeats.
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
        let reserved = engine::allocation::advisory(|| {
            engine::allocation::reserve_exact(&mut entries, rows.len())?;
            engine::allocation::reserve_exact(&mut held, text)?;
            engine::allocation::reserve_exact(&mut order, rows.len())?;
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
        entries.sort_unstable_by_key(|entry| entry.pointer);
        // A pointer occurring twice names no single held read.
        let mut index = 0;
        while index < entries.len() {
            let pointer = entries[index].pointer;
            let end = entries[index..]
                .iter()
                .position(|entry| entry.pointer != pointer)
                .map_or(entries.len(), |offset| index + offset);
            if end - index > 1 {
                entries.drain(index..end);
            } else {
                index = end;
            }
        }
        let mut order_entries = Vec::new();
        if engine::allocation::advisory(|| {
            engine::allocation::reserve_exact(&mut order_entries, order.len())
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
        // Characters certified since the set was chosen join it at a later
        // rediscovery: at once while nothing is carried, otherwise once they
        // are a thirty-second of the roster.
        self.promote =
            newly != 0 && (self.carried.is_empty() || newly.saturating_mul(32) >= rows.len());
        // The carried set must still name held rows.
        let entries = &self.entries;
        let before = self.carried.len();
        self.carried
            .retain(|pointer| entries.binary_search_by_key(pointer, |e| e.pointer).is_ok());
        if self.carried.len() != before {
            self.generation = self.generation.wrapping_add(1);
        }
    }
}

/// A carry version no other carry has held.
fn fresh_version() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// A fresh row equals a held one in every field the carry supplies.
fn same(row: &ccharacter::Row, bytes: &[u8], held: &ccharacter::Row, text: &[u8]) -> bool {
    let fresh = |start: usize, len: usize| bytes.get(start..start + len);
    let old = |start: usize, len: usize| text.get(start..start + len);
    row.id == held.id
        && row.name_token_id == held.name_token_id
        && row.gender == held.gender
        && row.template == held.template
        && row.template_identity == held.template_identity
        && fresh(row.name_start, row.name_len) == old(held.name_start, held.name_len)
        && fresh(row.token_start, row.token_len) == old(held.token_start, held.token_len)
        && fresh(row.template_file_start, row.template_file_len)
            == old(held.template_file_start, held.template_file_len)
}

#[cfg(test)]
mod order_tests {
    use super::*;

    /// A quiet hour's lookup at a held roster position takes that position's
    /// entry without a search; an occurrence the roster moved falls back to
    /// the search, and an unheld one finds nothing either way.
    #[test]
    fn held_positions_find_their_entries_and_moved_ones_search() {
        let mut carry = Carry {
            active: true,
            ..Carry::default()
        };
        let pointers = [0x3000, 0x1000, 0x2000];
        // The third occurrence holds no certificate.
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
        // The roster moved: another pointer at a held position, or a
        // position past the held order.
        assert_eq!(carry.entries[carry.position_at(1, 0x3000).unwrap()].id, 10);
        assert_eq!(carry.entries[carry.position_at(7, 0x1000).unwrap()].id, 11);
        assert_eq!(carry.position_at(0, 0x2000), None);
    }
}
