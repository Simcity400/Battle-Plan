//! PoliticalParties and ActiveIdeas rows held between hours per country
//! ([`crate::party_changes`]).
//!
//! The Politics walk still authenticates every country's political status
//! each hour and reads its party and idea array headers, every party and idea
//! pointer in them, the ruling party and each party's popularity.
//!
//! A country's PoliticalParties rows are carried, without reading its
//! parties' identities, ideology tokens, tags, names or default flags, only
//! when all of these hold at this boundary: the same country, political
//! status and party array data pointer and count as the held read, the same
//! party pointers in the same order, and no hooked writer touched any of
//! those parties since that read registered it (same watch epoch and
//! sequence; a load, gap, recovery, table change, poison or a running writer
//! revokes it). Every writer of those fields is a hooked site
//! ([`engine::political_party_mutations`]); popularity and the ruling flag
//! are read afresh, so they are held as zero.
//!
//! A country's ActiveIdeas rows are carried, without reading any idea, when
//! the same country, status, idea array header and idea pointers are those
//! of the held read, taken in the current epoch: a row is the slot's
//! `CIdea` definition token, which the idea database fixes for the session.
//!
//! Each hour also reads the carryable countries of its sampled slice
//! ([`crate::sampled_verify`], keyed by country id; every one under
//! `party_verify`) afresh, publishes the fresh rows and poisons reuse on a
//! difference. A fresh read whose pointers differ from the held read's is a
//! change, not a comparison. The store follows the Governance job's loan and
//! holds only the countries of the latest complete walk of each table.
//! Allocation is advisory: a country whose rows cannot be held is read afresh
//! next hour.
//!
//! The store also holds the latest complete walk's four politics tables for
//! a quiet politics hour ([`super::politics_hour`]), with every listed party
//! pointer in walk order, recorded here as each country's parties are read or
//! carried. A quiet hour neither begins nor finishes a walk, so the
//! per-country rows above stay as the latest walk left them.
use super::politics_hour::{QuietWalk, Visit};
use crate::{
    party_changes::{Changes, Counts},
    sampled_verify::Check,
};
use engine::{
    allocation,
    cpolitical_party::Row,
    cpolitical_status::{IdeaCarry, IdeaRow, Ideas, Parties, PartyCarry},
    walk::Outcome,
};
use std::ops::Range;

type Certified = (u64, u64);

const PARTIES: usize = 0;
const IDEAS: usize = 1;

#[derive(Clone, Debug)]
struct PartyEntry {
    parties: Parties,
    rows: Range<usize>,
    text: Range<usize>,
    /// Each listed party with its certificate, in array order.
    pointers: Range<usize>,
}

#[derive(Clone, Debug)]
struct IdeaEntry {
    ideas: Ideas,
    /// The status's certificate: no writer notifies it, so it changes only
    /// with the epoch.
    certificate: Certified,
    rows: Range<usize>,
    text: Range<usize>,
    /// Each slot's stored pointer, in array order.
    pointers: Range<usize>,
}

/// One walk's held parties.
#[derive(Default)]
struct HeldParties {
    /// Sorted by political status once the walk finishes.
    entries: Vec<PartyEntry>,
    rows: Vec<Row>,
    text: Vec<u8>,
    pointers: Vec<(usize, Certified)>,
}

impl HeldParties {
    fn clear(&mut self) {
        self.entries.clear();
        self.rows.clear();
        self.text.clear();
        self.pointers.clear();
    }

    fn find(&self, parties: &Parties) -> Option<usize> {
        let index = self
            .entries
            .binary_search_by_key(&parties.status, |entry| entry.parties.status)
            .ok()?;
        (self.entries[index].parties == *parties).then_some(index)
    }

    /// Holds one country; `false` when the storage could not grow, which
    /// only withholds the certificate.
    fn hold(
        &mut self,
        parties: Parties,
        rows: &[Row],
        text: &[u8],
        pointers: &[(usize, Certified)],
    ) -> bool {
        let stored = allocation::advisory(|| {
            allocation::reserve(&mut self.entries, 1)?;
            allocation::reserve(&mut self.rows, rows.len())?;
            allocation::reserve(&mut self.text, text.len())?;
            allocation::reserve(&mut self.pointers, pointers.len())?;
            std::io::Result::Ok(())
        });
        if stored.is_err() {
            return false;
        }
        let (row, byte, pointer) = (self.rows.len(), self.text.len(), self.pointers.len());
        // Popularity and the ruling flag are read every hour, never held.
        self.rows.extend(rows.iter().map(|row| Row {
            popularity: 0,
            is_ruling: 0,
            ..*row
        }));
        self.text.extend_from_slice(text);
        self.pointers.extend_from_slice(pointers);
        self.entries.push(PartyEntry {
            parties,
            rows: row..self.rows.len(),
            text: byte..self.text.len(),
            pointers: pointer..self.pointers.len(),
        });
        true
    }

    fn sort(&mut self) {
        sort_unique(&mut self.entries, |entry| entry.parties.status);
    }

    fn measure(&self) -> usize {
        self.entries.capacity() * size_of::<PartyEntry>()
            + self.rows.capacity() * size_of::<Row>()
            + self.text.capacity()
            + self.pointers.capacity() * size_of::<(usize, Certified)>()
    }
}

/// One walk's held ideas.
#[derive(Default)]
struct HeldIdeas {
    /// Sorted by political status once the walk finishes.
    entries: Vec<IdeaEntry>,
    rows: Vec<IdeaRow>,
    text: Vec<u8>,
    pointers: Vec<usize>,
}

impl HeldIdeas {
    fn clear(&mut self) {
        self.entries.clear();
        self.rows.clear();
        self.text.clear();
        self.pointers.clear();
    }

    fn find(&self, ideas: &Ideas) -> Option<usize> {
        let index = self
            .entries
            .binary_search_by_key(&ideas.status, |entry| entry.ideas.status)
            .ok()?;
        (self.entries[index].ideas == *ideas).then_some(index)
    }

    /// Holds one country; `false` when the storage could not grow.
    fn hold(
        &mut self,
        ideas: Ideas,
        certificate: Certified,
        rows: &[IdeaRow],
        text: &[u8],
        pointers: &[usize],
    ) -> bool {
        let stored = allocation::advisory(|| {
            allocation::reserve(&mut self.entries, 1)?;
            allocation::reserve(&mut self.rows, rows.len())?;
            allocation::reserve(&mut self.text, text.len())?;
            allocation::reserve(&mut self.pointers, pointers.len())?;
            std::io::Result::Ok(())
        });
        if stored.is_err() {
            return false;
        }
        let (row, byte, pointer) = (self.rows.len(), self.text.len(), self.pointers.len());
        self.rows.extend_from_slice(rows);
        self.text.extend_from_slice(text);
        self.pointers.extend_from_slice(pointers);
        self.entries.push(IdeaEntry {
            ideas,
            certificate,
            rows: row..self.rows.len(),
            text: byte..self.text.len(),
            pointers: pointer..self.pointers.len(),
        });
        true
    }

    fn sort(&mut self) {
        sort_unique(&mut self.entries, |entry| entry.ideas.status);
    }

    fn measure(&self) -> usize {
        self.entries.capacity() * size_of::<IdeaEntry>()
            + self.rows.capacity() * size_of::<IdeaRow>()
            + self.text.capacity()
            + self.pointers.capacity() * size_of::<usize>()
    }
}

/// Sorts held entries by status and drops every status occurring twice,
/// which names no single held read.
fn sort_unique<T>(entries: &mut Vec<T>, status: impl Fn(&T) -> usize) {
    entries.sort_unstable_by_key(&status);
    let mut index = 0;
    while index < entries.len() {
        let key = status(&entries[index]);
        let end = entries[index..]
            .iter()
            .position(|entry| status(entry) != key)
            .map_or(entries.len(), |offset| index + offset);
        if end - index > 1 {
            entries.drain(index..end);
        } else {
            index = end;
        }
    }
}

#[derive(Default)]
struct Held {
    parties: HeldParties,
    ideas: HeldIdeas,
}

/// The country being planned or read afresh, per table.
#[derive(Default)]
struct Pending {
    token: usize,
    /// The held entry whose certificates are all current.
    held: Option<usize>,
    /// The fresh read's pointers equal the held entry's so far.
    same: bool,
    /// Every fresh party registered.
    certified: bool,
    /// The read's party pointers and certificates, or idea pointers.
    parties: Vec<(usize, Certified)>,
    ideas: Vec<usize>,
    /// The ideas plan's status certificate.
    certificate: Option<Certified>,
    parties_header: Option<Parties>,
    ideas_header: Option<Ideas>,
}

/// Rows of the latest complete walk and the walk being made.
#[derive(Default)]
pub struct PartyStore {
    current: Held,
    previous: Held,
    pending: [Pending; 2],
    next_token: usize,
    counts: [Counts; 2],
    receivers: usize,
    /// Allocated bytes as of the last finish, for the game thread.
    bytes: usize,
    /// Set on the game thread; the next walk drops every held row.
    forgotten: bool,
    /// The latest complete walk's four politics tables.
    quiet: QuietWalk,
}

impl PartyStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Nothing held certifies anything: the next walk reads afresh.
    /// Allocation-free for the game thread.
    pub fn forget(&mut self) {
        self.forgotten = true;
    }

    /// As of the last finish, so the game thread need not measure.
    pub fn allocated_bytes(&self) -> usize {
        self.bytes
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn counts(&self) -> [Counts; 2] {
        self.counts
    }

    /// The latest walk hour's use of the political status domains.
    #[cfg(any(test, feature = "test-support"))]
    pub fn quiet_counts(&self) -> crate::political_status_changes::Counts {
        self.quiet.counts()
    }

    /// The held politics tables; `None` once forgotten.
    pub(super) fn quiet(&self) -> Option<&QuietWalk> {
        (!self.forgotten).then_some(&self.quiet)
    }

    pub(super) fn quiet_mut(&mut self) -> &mut QuietWalk {
        &mut self.quiet
    }

    /// Measures again after the politics tables changed.
    pub(super) fn remeasure(&mut self) {
        self.bytes = self.measure();
    }

    fn measure(&self) -> usize {
        let pending = self
            .pending
            .iter()
            .map(|pending| {
                pending.parties.capacity() * size_of::<(usize, Certified)>()
                    + pending.ideas.capacity() * size_of::<usize>()
            })
            .sum::<usize>();
        self.current.parties.measure()
            + self.current.ideas.measure()
            + self.previous.parties.measure()
            + self.previous.ideas.measure()
            + pending
            + self.quiet.measure()
    }

    fn begin(&mut self) {
        if std::mem::take(&mut self.forgotten) {
            self.current = Held::default();
            self.previous = Held::default();
            self.quiet = QuietWalk::default();
        }
        std::mem::swap(&mut self.current, &mut self.previous);
        self.current.parties.clear();
        self.current.ideas.clear();
        self.counts = [Counts::default(); 2];
        self.receivers = 0;
        self.quiet.begin();
    }

    /// Keeps each table's rows only when the walk read or carried every
    /// country of it completely, and publishes the counts.
    fn finish(&mut self, parties: bool, ideas: bool) {
        if parties {
            self.current.parties.sort();
        } else {
            self.current.parties.clear();
        }
        if ideas {
            self.current.ideas.sort();
        } else {
            self.current.ideas.clear();
        }
        self.previous.parties.clear();
        self.previous.ideas.clear();
        self.bytes = self.measure();
        crate::party_changes::record(&self.counts, (parties && ideas).then_some(self.receivers));
    }

    fn start(&mut self, table: usize) -> usize {
        self.next_token = self.next_token.wrapping_add(1);
        let pending = &mut self.pending[table];
        pending.token = self.next_token;
        pending.held = None;
        pending.same = true;
        pending.certified = true;
        pending.parties.clear();
        pending.ideas.clear();
        pending.certificate = None;
        pending.parties_header = None;
        pending.ideas_header = None;
        self.next_token
    }
}

/// One walk's view of the store and the observation that certifies reuse.
pub struct PoliticsReuse<'a> {
    store: &'a mut PartyStore,
    changes: &'a dyn Changes,
    /// Which carryable countries this hour reads afresh and compares.
    check: Check,
}

impl<'a> PoliticsReuse<'a> {
    pub fn begin(store: &'a mut PartyStore, changes: &'a dyn Changes, check: Check) -> Self {
        store.begin();
        Self {
            store,
            changes,
            check,
        }
    }

    /// Ends the walk: each flag says whether every country of that table was
    /// read or carried whole.
    pub fn finish(self, parties: bool, ideas: bool) {
        self.store.finish(parties, ideas);
    }

    fn pending(&mut self, table: usize, token: usize) -> Option<&mut Pending> {
        let pending = &mut self.store.pending[table];
        (pending.token == token && token != 0).then_some(pending)
    }

    /// Records a visited country's rows for a quiet politics hour, with the
    /// party pointers its PoliticalParties read or carry listed.
    pub(super) fn visit(&mut self, visit: Visit) {
        self.store.quiet.visit(visit);
    }
}

/// Appends without failing the read: a full buffer only withholds the
/// certificate.
fn push<T>(values: &mut Vec<T>, value: T) -> bool {
    if allocation::advisory(|| allocation::reserve(values, 1)).is_err() {
        return false;
    }
    values.push(value);
    true
}

impl PartyCarry for PoliticsReuse<'_> {
    /// Carryable when the held read of the same status and array header
    /// lists parties whose certificates are all still current, and carried
    /// without a read unless the hour's [`Check`] covers the country.
    fn plan(&mut self, parties: &Parties) -> (usize, Option<(usize, usize)>) {
        let token = self.store.start(PARTIES);
        self.store.quiet.start_country();
        let held = self.store.previous.parties.find(parties).filter(|&index| {
            let previous = &self.store.previous.parties;
            previous.pointers[previous.entries[index].pointers.clone()]
                .iter()
                .all(|&(party, certified)| self.changes.sequence(party) == Some(certified))
        });
        let pending = &mut self.store.pending[PARTIES];
        pending.held = held;
        pending.parties_header = Some(*parties);
        let carry = held
            .filter(|_| !self.check.covers(u64::from(parties.country)))
            .map(|index| {
                let entry = &self.store.previous.parties.entries[index];
                (entry.rows.len(), entry.text.len())
            });
        (token, carry)
    }

    fn same_parties(&mut self, token: usize, first: usize, chunk: &[usize]) -> bool {
        let Some(index) = self
            .pending(PARTIES, token)
            .and_then(|pending| pending.held)
        else {
            return false;
        };
        let previous = &self.store.previous.parties;
        let held = &previous.pointers[previous.entries[index].pointers.clone()];
        held.get(first..first + chunk.len()).is_some_and(|held| {
            held.iter()
                .zip(chunk)
                .all(|(&(party, _), &pointer)| party == pointer)
        })
    }

    /// The held rows of a carryable plan, held again for the next hour.
    fn carry(&mut self, token: usize, rows: &mut [Row], text: &mut [u8]) {
        let Some((index, header)) = self
            .pending(PARTIES, token)
            .and_then(|pending| Some((pending.held?, pending.parties_header?)))
        else {
            return;
        };
        let PartyStore {
            previous,
            current,
            counts,
            quiet,
            ..
        } = &mut *self.store;
        let entry = previous.parties.entries[index].clone();
        rows.copy_from_slice(&previous.parties.rows[entry.rows]);
        text.copy_from_slice(&previous.parties.text[entry.text]);
        counts[PARTIES].reused += 1;
        counts[PARTIES].reused_rows += rows.len() as u64;
        // The live pointers, which `same_parties` found equal.
        let pointers = &previous.parties.pointers[entry.pointers];
        quiet.carried_parties(pointers.iter().map(|&(party, _)| party));
        // A failed hold only withholds the next hour's certificate.
        let _ = current.parties.hold(header, rows, text, pointers);
    }

    /// Registers a party before its fields are read.
    fn party(&mut self, token: usize, party: usize) {
        let certificate = self.changes.sequence(party);
        self.store.receivers += 1;
        self.store.quiet.fresh_party(party);
        let PartyStore {
            previous, pending, ..
        } = &mut *self.store;
        let pending = &mut pending[PARTIES];
        if pending.token != token || token == 0 {
            return;
        }
        let ordinal = pending.parties.len();
        if let Some(index) = pending.held {
            let held = &previous.parties.pointers[previous.parties.entries[index].pointers.clone()];
            pending.same &= held.get(ordinal).is_some_and(|&(held, _)| held == party);
        }
        match certificate {
            Some(certificate) => {
                pending.certified &= push(&mut pending.parties, (party, certificate))
            }
            None => pending.certified = false,
        }
    }

    /// A fresh read: compared with the held rows when it lists the held
    /// parties (a difference poisons reuse) and held for the next hour.
    fn read(&mut self, token: usize, outcome: Outcome, rows: &[Row], text: &[u8]) {
        let changes = self.changes;
        let PartyStore {
            previous,
            current,
            pending,
            counts,
            quiet,
            ..
        } = &mut *self.store;
        let pending = &mut pending[PARTIES];
        if pending.token != token || token == 0 {
            return;
        }
        if matches!(outcome, Outcome::Rows { .. }) {
            quiet.read_parties();
        }
        let counts = &mut counts[PARTIES];
        counts.read += 1;
        counts.read_rows += rows.len() as u64;
        let complete = matches!(outcome, Outcome::Rows { .. });
        if let Some(index) = pending.held
            && pending.same
            && pending.parties.len() == rows.len()
        {
            let entry = &previous.parties.entries[index];
            if entry.pointers.len() == rows.len() {
                counts.verified += 1;
                let differs = !complete
                    || previous.parties.text[entry.text.clone()] != *text
                    || previous.parties.rows[entry.rows.clone()]
                        .iter()
                        .zip(rows)
                        .any(|(held, row)| {
                            *held
                                != Row {
                                    popularity: 0,
                                    is_ruling: 0,
                                    ..*row
                                }
                        });
                if differs {
                    counts.mismatches += 1;
                    changes.mismatch();
                }
            }
        }
        if complete
            && pending.certified
            && let Some(header) = pending.parties_header
        {
            let _ = current.parties.hold(header, rows, text, &pending.parties);
        }
    }
}

impl IdeaCarry for PoliticsReuse<'_> {
    /// Carryable when the held read of the same status and array header was
    /// taken in this epoch, and carried without a read unless the hour's
    /// [`Check`] covers the country.
    fn plan(&mut self, ideas: &Ideas) -> (usize, Option<(usize, usize)>) {
        let token = self.store.start(IDEAS);
        let certificate = self.changes.sequence(ideas.status);
        self.store.receivers += 1;
        let held = certificate.and_then(|certificate| {
            self.store.previous.ideas.find(ideas).filter(|&index| {
                self.store.previous.ideas.entries[index].certificate == certificate
            })
        });
        let pending = &mut self.store.pending[IDEAS];
        pending.held = held;
        pending.certificate = certificate;
        pending.ideas_header = Some(*ideas);
        let carry = held
            .filter(|_| !self.check.covers(u64::from(ideas.country)))
            .map(|index| {
                let entry = &self.store.previous.ideas.entries[index];
                (entry.rows.len(), entry.text.len())
            });
        (token, carry)
    }

    fn same_ideas(&mut self, token: usize, first: usize, chunk: &[usize]) -> bool {
        let Some(index) = self.pending(IDEAS, token).and_then(|pending| pending.held) else {
            return false;
        };
        let previous = &self.store.previous.ideas;
        previous.pointers[previous.entries[index].pointers.clone()]
            .get(first..first + chunk.len())
            .is_some_and(|held| held == chunk)
    }

    /// The held rows of a carryable plan, held again for the next hour.
    fn carry(&mut self, token: usize, rows: &mut [IdeaRow], text: &mut [u8]) {
        let Some((index, header, certificate)) = self
            .pending(IDEAS, token)
            .and_then(|pending| Some((pending.held?, pending.ideas_header?, pending.certificate?)))
        else {
            return;
        };
        let PartyStore {
            previous,
            current,
            counts,
            ..
        } = &mut *self.store;
        let entry = previous.ideas.entries[index].clone();
        rows.copy_from_slice(&previous.ideas.rows[entry.rows]);
        text.copy_from_slice(&previous.ideas.text[entry.text]);
        counts[IDEAS].reused += 1;
        counts[IDEAS].reused_rows += rows.len() as u64;
        let _ = current.ideas.hold(
            header,
            certificate,
            rows,
            text,
            &previous.ideas.pointers[entry.pointers],
        );
    }

    /// Records a slot's stored pointer before the idea is read.
    fn idea(&mut self, token: usize, slot: usize) {
        let PartyStore {
            previous, pending, ..
        } = &mut *self.store;
        let pending = &mut pending[IDEAS];
        if pending.token != token || token == 0 {
            return;
        }
        let ordinal = pending.ideas.len();
        if let Some(index) = pending.held {
            let held = &previous.ideas.pointers[previous.ideas.entries[index].pointers.clone()];
            pending.same &= held.get(ordinal) == Some(&slot);
        }
        pending.certified &= push(&mut pending.ideas, slot);
    }

    /// A fresh read: compared with the held rows when it lists the held
    /// slots (a difference poisons reuse) and held for the next hour.
    fn read(&mut self, token: usize, outcome: Outcome, rows: &[IdeaRow], text: &[u8]) {
        let changes = self.changes;
        let PartyStore {
            previous,
            current,
            pending,
            counts,
            ..
        } = &mut *self.store;
        let pending = &mut pending[IDEAS];
        if pending.token != token || token == 0 {
            return;
        }
        let counts = &mut counts[IDEAS];
        counts.read += 1;
        counts.read_rows += rows.len() as u64;
        let complete = matches!(outcome, Outcome::Rows { .. });
        if let Some(index) = pending.held
            && pending.same
            && pending.ideas.len() == previous.ideas.entries[index].pointers.len()
        {
            let entry = &previous.ideas.entries[index];
            counts.verified += 1;
            if !complete
                || previous.ideas.rows[entry.rows.clone()] != *rows
                || previous.ideas.text[entry.text.clone()] != *text
            {
                counts.mismatches += 1;
                changes.mismatch();
            }
        }
        if complete
            && pending.certified
            && let (Some(header), Some(certificate)) = (pending.ideas_header, pending.certificate)
        {
            let _ = current
                .ideas
                .hold(header, certificate, rows, text, &pending.ideas);
        }
    }
}
