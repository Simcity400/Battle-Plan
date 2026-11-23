use super::politics_hour::{QuietWalk, Visit};
use crate::party_changes::{Changes, Counts};
use engine_allocation as allocation;
use engine_native_politics::{
    cpolitical_party::Row,
    cpolitical_status::{
        IdeaCarry,
        IdeaRow,
        Ideas,
        Parties,
        PartyCarry,
    },
};
use engine_native::walk::Outcome;
use std::ops::Range;

type Certified = (u64, u64);

const PARTIES: usize = 0;
const IDEAS: usize = 1;

#[derive(Clone, Debug)]
struct PartyEntry {
    parties: Parties,
    rows: Range<usize>,
    text: Range<usize>,
    pointers: Range<usize>,
}

#[derive(Clone, Debug)]
struct IdeaEntry {
    ideas: Ideas,
    certificate: Certified,
    rows: Range<usize>,
    text: Range<usize>,
    pointers: Range<usize>,
}

#[derive(Default)]
struct HeldParties {
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

#[derive(Default)]
struct HeldIdeas {
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

#[derive(Default)]
struct Pending {
    token: usize,
    held: Option<usize>,
    certified: bool,
    parties: Vec<(usize, Certified)>,
    ideas: Vec<usize>,
    certificate: Option<Certified>,
    parties_header: Option<Parties>,
    ideas_header: Option<Ideas>,
}

#[derive(Default)]
pub struct PartyStore {
    current: Held,
    previous: Held,
    pending: [Pending; 2],
    next_token: usize,
    counts: [Counts; 2],
    receivers: usize,
    bytes: usize,
    forgotten: bool,
    quiet: QuietWalk,
}

impl PartyStore {
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
    pub fn counts(&self) -> [Counts; 2] {
        self.counts
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn quiet_counts(&self) -> crate::political_status_changes::Counts {
        self.quiet.counts()
    }

    pub(super) fn quiet(&self) -> Option<&QuietWalk> {
        (!self.forgotten).then_some(&self.quiet)
    }

    pub(super) fn quiet_mut(&mut self) -> &mut QuietWalk {
        &mut self.quiet
    }

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
        pending.certified = true;
        pending.parties.clear();
        pending.ideas.clear();
        pending.certificate = None;
        pending.parties_header = None;
        pending.ideas_header = None;
        self.next_token
    }
}

pub struct PoliticsReuse<'a> {
    store: &'a mut PartyStore,
    changes: &'a dyn Changes,
}

impl<'a> PoliticsReuse<'a> {
    pub fn begin(store: &'a mut PartyStore, changes: &'a dyn Changes) -> Self {
        store.begin();
        Self { store, changes }
    }

    pub fn finish(self, parties: bool, ideas: bool) {
        self.store.finish(parties, ideas);
    }

    fn pending(&mut self, table: usize, token: usize) -> Option<&mut Pending> {
        let pending = &mut self.store.pending[table];
        (pending.token == token && token != 0).then_some(pending)
    }

    pub(super) fn visit(&mut self, visit: Visit) {
        self.store.quiet.visit(visit);
    }
}

fn push<T>(values: &mut Vec<T>, value: T) -> bool {
    if allocation::advisory(|| allocation::reserve(values, 1)).is_err() {
        return false;
    }
    values.push(value);
    true
}

impl PartyCarry for PoliticsReuse<'_> {
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
        let carry = held.map(|index| {
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
        let pointers = &previous.parties.pointers[entry.pointers];
        quiet.carried_parties(pointers.iter().map(|&(party, _)| party));
        let _ = current.parties.hold(header, rows, text, pointers);
    }

    fn party(&mut self, token: usize, party: usize) {
        let certificate = self.changes.sequence(party);
        self.store.receivers += 1;
        self.store.quiet.fresh_party(party);
        let pending = &mut self.store.pending[PARTIES];
        if pending.token != token || token == 0 {
            return;
        }
        match certificate {
            Some(certificate) => {
                pending.certified &= push(&mut pending.parties, (party, certificate))
            }
            None => pending.certified = false,
        }
    }

    fn read(&mut self, token: usize, outcome: Outcome, rows: &[Row], text: &[u8]) {
        let PartyStore {
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
        if complete
            && pending.certified
            && let Some(header) = pending.parties_header
        {
            let _ = current.parties.hold(header, rows, text, &pending.parties);
        }
    }
}

impl IdeaCarry for PoliticsReuse<'_> {
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
        let carry = held.map(|index| {
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

    fn idea(&mut self, token: usize, slot: usize) {
        let pending = &mut self.store.pending[IDEAS];
        if pending.token != token || token == 0 {
            return;
        }
        pending.certified &= push(&mut pending.ideas, slot);
    }

    fn read(&mut self, token: usize, outcome: Outcome, rows: &[IdeaRow], text: &[u8]) {
        let PartyStore {
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
