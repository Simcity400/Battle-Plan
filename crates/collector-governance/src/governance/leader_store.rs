use crate::party_leader_changes::{Changes, Counts};
use engine_allocation as allocation;
use engine_native_politics::{
    cpolitical_party::LeaderRow,
    cpolitical_status::{LeaderCarry, Parties},
};
use std::ops::Range;

type Certified = (u64, u64);

#[derive(Clone, Debug)]
struct Entry {
    parties: Parties,
    rows: Range<usize>,
    party_certificates: Range<usize>,
    characters: Range<usize>,
}

#[derive(Default)]
struct Held {
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

#[derive(Default)]
struct Pending {
    token: usize,
    parties: Option<Parties>,
    held: Option<usize>,
    certified: bool,
    party_certificates: Vec<(usize, Certified)>,
    characters: Vec<(usize, Certified)>,
}

#[derive(Default)]
pub struct LeaderStore {
    current: Held,
    previous: Held,
    pending: Pending,
    planned: usize,
    parties: usize,
    counts: Counts,
    bytes: usize,
    forgotten: bool,
}

impl LeaderStore {
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

    pub(super) fn held(&self) -> Option<(usize, &[(usize, Certified)])> {
        (!self.forgotten).then_some((self.current.entries.len(), &self.current.characters))
    }
}

pub struct LeaderReuse<'a> {
    store: &'a mut LeaderStore,
    changes: &'a dyn Changes,
}

impl<'a> LeaderReuse<'a> {
    pub fn begin(store: &'a mut LeaderStore, changes: &'a dyn Changes) -> Self {
        if std::mem::take(&mut store.forgotten) {
            store.current.clear();
            store.previous.clear();
        }
        std::mem::swap(&mut store.current, &mut store.previous);
        store.current.clear();
        store.counts = Counts::default();
        store.planned = 0;
        store.parties = 0;
        Self { store, changes }
    }

    pub fn finish(self, complete: bool) {
        let store = self.store;
        if complete {
            let entries = &mut store.current.entries;
            entries.sort_unstable_by_key(|entry| entry.parties.status);
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
        (
            token,
            held.map(|index| self.store.previous.entries[index].rows.len()),
        )
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
