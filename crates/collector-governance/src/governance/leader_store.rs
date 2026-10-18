//! PartyLeaders rows held between hours per country
//! ([`crate::party_leader_changes`]).
//!
//! The Politics walk still authenticates every country's political status and
//! reads its party array header each hour. A country's held rows are
//! carried, without reading its parties, leaders or characters, only when all
//! of these hold at this boundary: the same country, political status and
//! party array data pointer and count as the held read; no hooked writer
//! touched any party that read listed since it registered it (same watch
//! epoch and sequence; a load, gap, recovery, table change, poison or a
//! running writer revokes it); and every listed leader's character keeps the
//! certificate that read took ([`crate::character_changes`], whose sites
//! cover every write of a character's leader map). A party listed at a
//! destroyed party's address carries that destructor's notification, and a
//! leader's character link and id are written only on a new object a hooked
//! insert lists. Registration precedes every read it certifies: a party
//! before its leader vector, a character before its identity and leader map.
//!
//! Each hour also reads the carryable countries of its sampled slice
//! ([`crate::sampled_verify`], keyed by country id; every one under
//! `party_leader_verify`) afresh, publishes the fresh rows and poisons reuse
//! on a difference. The store follows the Governance job's loan and
//! holds only the countries of the latest complete walk. Allocation is
//! advisory: a country whose rows cannot be held is read afresh next hour.
use crate::{
    party_leader_changes::{Changes, Counts},
    sampled_verify::Check,
};
use engine_allocation as allocation;
use engine_native::{
    cpolitical_party::LeaderRow, cpolitical_status::LeaderCarry, cpolitical_status::Parties,
};
use std::ops::Range;

type Certified = (u64, u64);

#[derive(Clone, Debug)]
struct Entry {
    parties: Parties,
    rows: Range<usize>,
    /// Each listed party with its certificate.
    party_certificates: Range<usize>,
    /// Each listed leader's character with its certificate.
    characters: Range<usize>,
}

/// One walk's held countries.
#[derive(Default)]
struct Held {
    /// Sorted by political status once the walk finishes.
    entries: Vec<Entry>,
    rows: Vec<LeaderRow>,
    parties: Vec<(usize, Certified)>,
    characters: Vec<(usize, Certified)>,
}

impl Held {
    fn clear(&mut self) {
        self.entries.clear();
        self.rows.clear();
        self.parties.clear();
        self.characters.clear();
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
        rows: &[LeaderRow],
        party_certificates: &[(usize, Certified)],
        characters: &[(usize, Certified)],
    ) -> bool {
        let stored = allocation::advisory(|| {
            allocation::reserve(&mut self.entries, 1)?;
            allocation::reserve(&mut self.rows, rows.len())?;
            allocation::reserve(&mut self.parties, party_certificates.len())?;
            allocation::reserve(&mut self.characters, characters.len())?;
            std::io::Result::Ok(())
        });
        if stored.is_err() {
            return false;
        }
        let (row, party, character) = (self.rows.len(), self.parties.len(), self.characters.len());
        self.rows.extend_from_slice(rows);
        self.parties.extend_from_slice(party_certificates);
        self.characters.extend_from_slice(characters);
        self.entries.push(Entry {
            parties,
            rows: row..self.rows.len(),
            party_certificates: party..self.parties.len(),
            characters: character..self.characters.len(),
        });
        true
    }

    fn measure(&self) -> usize {
        self.entries.capacity() * size_of::<Entry>()
            + self.rows.capacity() * size_of::<LeaderRow>()
            + (self.parties.capacity() + self.characters.capacity())
                * size_of::<(usize, Certified)>()
    }
}

/// The country being read afresh.
#[derive(Default)]
struct Pending {
    token: usize,
    parties: Option<Parties>,
    /// The carryable held entry this read verifies.
    held: Option<usize>,
    /// `false` once a party or character could not be registered.
    certified: bool,
    party_certificates: Vec<(usize, Certified)>,
    characters: Vec<(usize, Certified)>,
}

/// Rows of the latest complete walk and the walk being made.
#[derive(Default)]
pub struct LeaderStore {
    current: Held,
    previous: Held,
    pending: Pending,
    planned: usize,
    parties: usize,
    counts: Counts,
    /// Allocated bytes as of the last finish, for the game thread.
    bytes: usize,
    /// Set on the game thread; the next walk drops every held row.
    forgotten: bool,
}

impl LeaderStore {
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
    pub fn counts(&self) -> Counts {
        self.counts
    }

    /// The latest complete walk's held countries, and each listed leader's
    /// character with the certificate its read took, for a quiet politics
    /// hour ([`super::politics_hour`]); `None` once forgotten. A quiet hour
    /// leaves the store as it is, so the next walk carries from it.
    pub(super) fn held(&self) -> Option<(usize, &[(usize, Certified)])> {
        (!self.forgotten).then_some((self.current.entries.len(), &self.current.characters))
    }
}

/// One walk's view of the store and the observation that certifies reuse.
pub struct LeaderReuse<'a> {
    store: &'a mut LeaderStore,
    changes: &'a dyn Changes,
    /// Which carryable countries this hour reads afresh and compares, by
    /// country id.
    pub check: Check,
}

impl<'a> LeaderReuse<'a> {
    pub fn begin(store: &'a mut LeaderStore, changes: &'a dyn Changes, check: Check) -> Self {
        if std::mem::take(&mut store.forgotten) {
            store.current.clear();
            store.previous.clear();
        }
        std::mem::swap(&mut store.current, &mut store.previous);
        store.current.clear();
        store.counts = Counts::default();
        store.planned = 0;
        store.parties = 0;
        Self {
            store,
            changes,
            check,
        }
    }

    /// Keeps this walk's countries only when every country was read or
    /// carried completely, and publishes its counts.
    pub fn finish(self, complete: bool) {
        let store = self.store;
        if complete {
            let entries = &mut store.current.entries;
            entries.sort_unstable_by_key(|entry| entry.parties.status);
            // A status occurring twice names no single held read.
            let mut index = 0;
            while index < entries.len() {
                let status = entries[index].parties.status;
                let end = entries[index..]
                    .iter()
                    .position(|entry| entry.parties.status != status)
                    .map_or(entries.len(), |offset| index + offset);
                if end - index > 1 {
                    entries.drain(index..end);
                } else {
                    index = end;
                }
            }
        } else {
            store.current.clear();
        }
        store.previous.clear();
        store.bytes = store.current.measure()
            + store.previous.measure()
            + (store.pending.party_certificates.capacity() + store.pending.characters.capacity())
                * size_of::<(usize, Certified)>();
        crate::party_leader_changes::record(&store.counts, complete.then_some(store.parties));
    }

    /// Whether a held entry's parties and characters are all unchanged.
    fn current(&self, index: usize) -> bool {
        let previous = &self.store.previous;
        let entry = &previous.entries[index];
        previous.parties[entry.party_certificates.clone()]
            .iter()
            .all(|&(party, certified)| self.changes.party(party) == Some(certified))
            && previous.characters[entry.characters.clone()]
                .iter()
                .all(|&(character, certified)| self.changes.character(character) == Some(certified))
    }

    fn certify(&mut self, certified: Option<Certified>, object: usize, party: bool) {
        let pending = &mut self.store.pending;
        let Some(certified) = certified else {
            pending.certified = false;
            return;
        };
        let list = if party {
            &mut pending.party_certificates
        } else {
            &mut pending.characters
        };
        if allocation::advisory(|| allocation::reserve(list, 1)).is_err() {
            pending.certified = false;
            return;
        }
        list.push((object, certified));
    }
}

impl LeaderCarry for LeaderReuse<'_> {
    fn plan(&mut self, parties: &Parties) -> (usize, Option<usize>) {
        self.store.planned += 1;
        self.store.parties += parties.count as usize;
        let token = self.store.planned;
        let held = self
            .store
            .previous
            .find(parties)
            .filter(|&index| self.current(index));
        let pending = &mut self.store.pending;
        pending.token = token;
        pending.parties = Some(*parties);
        pending.held = held;
        pending.certified = true;
        pending.party_certificates.clear();
        pending.characters.clear();
        match held {
            Some(index) if !self.check.covers(u64::from(parties.country)) => {
                (token, Some(self.store.previous.entries[index].rows.len()))
            }
            _ => (token, None),
        }
    }

    fn carry(&mut self, token: usize, rows: &mut [LeaderRow]) {
        let store = &mut *self.store;
        let (Some(parties), Some(index)) = (store.pending.parties.take(), store.pending.held)
        else {
            return;
        };
        if store.pending.token != token {
            return;
        }
        let LeaderStore {
            previous,
            current,
            counts,
            ..
        } = store;
        let entry = &previous.entries[index];
        rows.copy_from_slice(&previous.rows[entry.rows.clone()]);
        counts.reused += 1;
        counts.reused_rows += rows.len() as u64;
        // A failed hold only withholds the next hour's certificate.
        let _ = current.hold(
            parties,
            rows,
            &previous.parties[entry.party_certificates.clone()],
            &previous.characters[entry.characters.clone()],
        );
    }

    fn party(&mut self, token: usize, party: usize) {
        if self.store.pending.token == token {
            let certified = self.changes.party(party);
            self.certify(certified, party, true);
        }
    }

    fn character(&mut self, token: usize, character: usize) {
        if self.store.pending.token == token {
            let certified = self.changes.character(character);
            self.certify(certified, character, false);
        }
    }

    fn read(&mut self, token: usize, rows: &[LeaderRow]) {
        let store = &mut *self.store;
        store.counts.read += 1;
        store.counts.read_rows += rows.len() as u64;
        if store.pending.token != token {
            return;
        }
        let Some(parties) = store.pending.parties.take() else {
            return;
        };
        if let Some(index) = store.pending.held {
            store.counts.verified += 1;
            let entry = &store.previous.entries[index];
            if store.previous.rows[entry.rows.clone()] != *rows {
                store.counts.mismatches += 1;
                self.changes.mismatch();
            }
        }
        if store.pending.certified
            && store.pending.party_certificates.len() == parties.count as usize
        {
            let Pending {
                party_certificates,
                characters,
                ..
            } = &store.pending;
            let _ = store
                .current
                .hold(parties, rows, party_certificates, characters);
        }
    }
}
