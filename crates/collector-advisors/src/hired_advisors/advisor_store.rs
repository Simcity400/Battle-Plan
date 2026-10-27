//! AvailableAdvisors and AdvisorTokens rows held between hours
//! ([`crate::advisor_pool_changes`]).
//!
//! AvailableAdvisors is read once a game day per country, at its owner
//! slice (`slicing::AVAILABLE_ADVISORS`); the walk reads and authenticates
//! each country of the hour's selection, its `CCountryCharacters` and the
//! available array's header. A pool's held rows are carried, without reading
//! its advisors, characters or idea bytes, only when all of these hold at
//! this boundary: the same country, `CCountryCharacters` and array data
//! pointer and count as the held read; no hooked writer touched the pool
//! since that read registered it (same watch epoch and sequence; a load,
//! gap, recovery, table change, poison, remove_advisor_role or a running
//! writer revokes it); and every listed advisor's character has the
//! certificate the held read took ([`crate::character_changes`], whose sites
//! cover every write of a listed advisor's character link and idea and of the
//! character's identity). A fresh read holds its rows only when no character
//! writer ran during it. Each hour of the day holds its own latest complete
//! walk, so a pool's next read, a day later at the same slice, finds the read
//! it may carry.
//!
//! The AdvisorTokens table is read every hour and carried whole when its
//! manager and table header are those of the held read and no hooked writer
//! touched the manager since. HiredAdvisors' latest complete walk is held
//! here too ([`super::appointed`]).
//!
//! The store follows the Slow job's loan and holds only the latest complete
//! walks, so its size follows live demand. Allocation is advisory: what
//! cannot be held is read afresh at its next read.
use crate::advisor_pool_changes::{Changes, Counts, Table};
use engine_allocation as allocation;
use engine_characters::{advisor_tokens, advisor_tokens::TokenCarry};
use engine_native::{
    ccountry_characters::AvailableCarry, ccountry_characters::Pool, ccountry_characters::Row,
};
use std::ops::Range;

type Certified = (u64, u64);
/// A planned pool: its token, certificate, the held entry it carries and
/// the character domain's quiet state before its read.
type Planned = (
    usize,
    Option<PoolCertificate>,
    Option<usize>,
    Option<Certified>,
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PoolCertificate {
    epoch: u64,
    sequence: u64,
    pool: Pool,
}

#[derive(Clone, Debug)]
struct Entry {
    certificate: PoolCertificate,
    rows: Range<usize>,
    bytes: Range<usize>,
    characters: Range<usize>,
}

/// One walk's held pools: rows with idea offsets relative to their pool's
/// bytes, and each row's character with its certificate.
#[derive(Default)]
struct Held {
    /// Sorted by `CCountryCharacters` once the walk finishes.
    entries: Vec<Entry>,
    rows: Vec<Row>,
    bytes: Vec<u8>,
    characters: Vec<(usize, Certified)>,
}

impl Held {
    fn clear(&mut self) {
        self.entries.clear();
        self.rows.clear();
        self.bytes.clear();
        self.characters.clear();
    }

    fn find(&self, certificate: &PoolCertificate) -> Option<usize> {
        let index = self
            .entries
            .binary_search_by_key(&certificate.pool.object, |entry| {
                entry.certificate.pool.object
            })
            .ok()?;
        (self.entries[index].certificate == *certificate).then_some(index)
    }

    /// Holds one pool; `false` when the storage could not grow, which only
    /// withholds the certificate.
    fn hold(
        &mut self,
        certificate: PoolCertificate,
        rows: &[Row],
        bytes: &[u8],
        bytes_start: usize,
        characters: &[(usize, Certified)],
    ) -> bool {
        let stored = allocation::advisory(|| {
            allocation::reserve(&mut self.entries, 1)?;
            allocation::reserve(&mut self.rows, rows.len())?;
            allocation::reserve(&mut self.bytes, bytes.len())?;
            allocation::reserve(&mut self.characters, characters.len())?;
            std::io::Result::Ok(())
        });
        if stored.is_err() {
            return false;
        }
        let (row, byte, character) = (self.rows.len(), self.bytes.len(), self.characters.len());
        self.rows.extend(rows.iter().map(|row| Row {
            idea_start: row.idea_start - bytes_start,
            slot_start: row.slot_start - bytes_start,
            ..*row
        }));
        self.bytes.extend_from_slice(bytes);
        self.characters.extend_from_slice(characters);
        self.entries.push(Entry {
            certificate,
            rows: row..self.rows.len(),
            bytes: byte..self.bytes.len(),
            characters: character..self.characters.len(),
        });
        true
    }

    fn measure(&self) -> usize {
        self.entries.capacity() * size_of::<Entry>()
            + self.rows.capacity() * size_of::<Row>()
            + self.bytes.capacity()
            + self.characters.capacity() * size_of::<(usize, Certified)>()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TokenCertificate {
    epoch: u64,
    sequence: u64,
    manager: usize,
    header: [u8; advisor_tokens::HEADER],
}

#[derive(Default)]
struct Tokens {
    certificate: Option<TokenCertificate>,
    rows: Vec<advisor_tokens::Row>,
    bytes: Vec<u8>,
}

/// Hours in a day: AvailableAdvisors reads each pool at one of them.
const SLICES: usize = replay_data::cadence::STAGGER_PERIOD as usize;

/// The latest complete walks and the walk being made.
#[derive(Default)]
pub struct AdvisorStore {
    /// Each hour of the day's latest complete walk, by `hour % SLICES`. A
    /// pool is read at its country's slice hour, so the read its next walk
    /// may carry is the one its slice held a day earlier; a full hour's walk
    /// holds every pool in its own hour's place.
    slices: [Held; SLICES],
    /// The place in the day of the walk being made.
    slice: usize,
    current: Held,
    previous: Held,
    /// The pool being planned or read afresh.
    planned: Option<Planned>,
    /// The characters the pool read afresh registered so far.
    pending: Option<Vec<(usize, Certified)>>,
    pools: usize,
    counts: Counts,
    tokens: Tokens,
    token_counts: Counts,
    /// HiredAdvisors' latest complete walk ([`super::appointed`]).
    appointed: super::appointed::Appointed,
    /// The character records' latest complete walk ([`super::records`]).
    records: super::records::Records,
    /// Allocated bytes as of the last finish, for the game thread.
    bytes: usize,
    /// Set on the game thread; the next walks drop every held row.
    forgotten: bool,
}

impl AdvisorStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Nothing held certifies anything: the next walks read afresh.
    /// Allocation-free for the game thread.
    pub fn forget(&mut self) {
        self.forgotten = true;
    }

    /// As of the last finish, so the game thread need not measure.
    pub fn allocated_bytes(&self) -> usize {
        self.bytes
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn counts(&self) -> (Counts, Counts) {
        (self.counts, self.token_counts)
    }

    /// HiredAdvisors' held walk, after a forgotten store dropped every held
    /// row.
    pub fn appointed(&mut self) -> &mut super::appointed::Appointed {
        self.take_forgotten();
        &mut self.appointed
    }

    /// The character records' held walk, after a forgotten store dropped
    /// every held row.
    pub fn records(&mut self) -> &mut super::records::Records {
        self.take_forgotten();
        &mut self.records
    }

    fn take_forgotten(&mut self) {
        if std::mem::take(&mut self.forgotten) {
            self.current.clear();
            self.previous.clear();
            for held in &mut self.slices {
                held.clear();
            }
            self.tokens = Tokens::default();
            self.appointed.forget();
            self.records.forget();
        }
    }

    fn measure(&mut self) {
        self.bytes = self.current.measure()
            + self.previous.measure()
            + self.slices.iter().map(Held::measure).sum::<usize>()
            + self.pending.as_ref().map_or(0, |pending| {
                pending.capacity() * size_of::<(usize, Certified)>()
            })
            + self.tokens.rows.capacity() * size_of::<advisor_tokens::Row>()
            + self.tokens.bytes.capacity()
            + self.appointed.measure()
            + self.records.measure();
    }
}

/// One AvailableAdvisors walk's view of the store.
pub struct PoolReuse<'a> {
    store: &'a mut AdvisorStore,
    changes: &'a dyn Changes,
}

impl<'a> PoolReuse<'a> {
    /// A walk at `hour`, which carries from that hour of the day's held walk
    /// and replaces it.
    pub fn begin(store: &'a mut AdvisorStore, changes: &'a dyn Changes, hour: i32) -> Self {
        store.take_forgotten();
        store.slice = hour.rem_euclid(SLICES as i32) as usize;
        std::mem::swap(&mut store.previous, &mut store.slices[store.slice]);
        store.slices[store.slice].clear();
        store.current.clear();
        store.counts = Counts::default();
        store.pools = 0;
        store.planned = None;
        Self { store, changes }
    }

    /// Keeps this walk's pools, in its hour's place, only when every pool
    /// was read or carried completely, and publishes its counts. `census` is
    /// the number of countries the hour's slice scan saw (zero without a
    /// scan): the pool watches are sized for every pool of the day, not only
    /// the slice this walk read.
    pub fn finish(self, complete: bool, census: usize) {
        let store = self.store;
        if complete {
            let entries = &mut store.current.entries;
            entries.sort_unstable_by_key(|entry| entry.certificate.pool.object);
            // An object occurring twice names no single held read.
            let mut index = 0;
            while index < entries.len() {
                let object = entries[index].certificate.pool.object;
                let end = entries[index..]
                    .iter()
                    .position(|entry| entry.certificate.pool.object != object)
                    .map_or(entries.len(), |offset| index + offset);
                if end - index > 1 {
                    entries.drain(index..end);
                } else {
                    index = end;
                }
            }
            std::mem::swap(&mut store.current, &mut store.slices[store.slice]);
        }
        store.current.clear();
        store.previous.clear();
        store.planned = None;
        store.measure();
        crate::advisor_pool_changes::record(
            Table::AvailableAdvisors,
            &store.counts,
            complete.then_some(store.pools.max(census)),
        );
    }
}

impl AvailableCarry for PoolReuse<'_> {
    /// Registration of the pool, and the character domain's quiet state,
    /// precede every read they certify.
    fn plan(&mut self, pool: &Pool) -> (usize, Option<(usize, usize)>) {
        let store = &mut *self.store;
        store.pools += 1;
        let token = store.pools;
        let certificate =
            self.changes
                .sequence(pool.object)
                .map(|(epoch, sequence)| PoolCertificate {
                    epoch,
                    sequence,
                    pool: *pool,
                });
        let held = certificate
            .and_then(|certificate| store.previous.find(&certificate))
            .filter(|&index| {
                let entry = &store.previous.entries[index];
                store.previous.characters[entry.characters.clone()]
                    .iter()
                    .all(|&(character, certified)| {
                        self.changes.character(character) == Some(certified)
                    })
            });
        if let Some(index) = held {
            let entry = &store.previous.entries[index];
            store.planned = Some((token, certificate, Some(index), None));
            return (token, Some((entry.rows.len(), entry.bytes.len())));
        }
        store.planned = Some((token, certificate, None, self.changes.characters_quiet()));
        let pending = store.pending.get_or_insert_with(Vec::new);
        pending.clear();
        (token, None)
    }

    fn carry(&mut self, token: usize, rows: &mut [Row], bytes: &mut [u8], bytes_start: usize) {
        let store = &mut *self.store;
        let Some((planned, Some(certificate), Some(index), _)) = store.planned.take() else {
            return;
        };
        if planned != token {
            return;
        }
        let AdvisorStore {
            previous,
            current,
            counts,
            ..
        } = store;
        let entry = &previous.entries[index];
        for (target, row) in rows.iter_mut().zip(&previous.rows[entry.rows.clone()]) {
            *target = Row {
                idea_start: row.idea_start + bytes_start,
                slot_start: row.slot_start + bytes_start,
                ..*row
            };
        }
        bytes.copy_from_slice(&previous.bytes[entry.bytes.clone()]);
        counts.reused += 1;
        counts.reused_rows += rows.len() as u64;
        // A failed hold only withholds the next hour's certificate.
        let _ = current.hold(
            certificate,
            rows,
            bytes,
            bytes_start,
            &previous.characters[entry.characters.clone()],
        );
    }

    fn advisor(&mut self, _token: usize, character: usize) {
        let certified = self.changes.character(character);
        let store = &mut *self.store;
        let held = match (store.pending.as_mut(), certified) {
            (Some(pending), Some(certified)) => allocation::advisory(|| {
                allocation::reserve(pending, 1)?;
                pending.push((character, certified));
                std::io::Result::Ok(())
            })
            .is_ok(),
            _ => false,
        };
        if !held && let Some((_, certificate, _, _)) = store.planned.as_mut() {
            // This pool cannot be certified; it is read afresh next hour.
            *certificate = None;
        }
    }

    fn read(&mut self, token: usize, rows: &[Row], bytes: &[u8], bytes_start: usize) {
        let quiet = self.changes.characters_quiet();
        let store = &mut *self.store;
        store.counts.read += 1;
        store.counts.read_rows += rows.len() as u64;
        let Some((planned, certificate, _, before)) = store.planned.take() else {
            return;
        };
        if planned != token {
            return;
        }
        let pending = store.pending.take().unwrap_or_default();
        if let Some(certificate) = certificate
            && before.is_some()
            && before == quiet
            && pending.len() == rows.len()
        {
            let _ = store
                .current
                .hold(certificate, rows, bytes, bytes_start, &pending);
        }
        store.pending = Some(pending);
    }
}

/// One AdvisorTokens walk's view of the store.
pub struct TokenReuse<'a> {
    store: &'a mut AdvisorStore,
    changes: &'a dyn Changes,
    /// The certificate this walk registered, held with a complete read.
    certificate: Option<TokenCertificate>,
    carried: bool,
    read: Option<bool>,
}

impl<'a> TokenReuse<'a> {
    pub fn begin(store: &'a mut AdvisorStore, changes: &'a dyn Changes) -> Self {
        store.take_forgotten();
        store.token_counts = Counts::default();
        Self {
            store,
            changes,
            certificate: None,
            carried: false,
            read: None,
        }
    }

    /// Keeps the table only when it was read or carried completely, and
    /// publishes its counts.
    pub fn finish(self, complete: bool) {
        let store = self.store;
        if !complete || !(self.carried || self.read == Some(true)) {
            store.tokens.certificate = None;
        }
        store.measure();
        crate::advisor_pool_changes::record(Table::AdvisorTokens, &store.token_counts, None);
    }
}

impl TokenCarry for TokenReuse<'_> {
    fn plan(
        &mut self,
        manager: usize,
        header: &[u8; advisor_tokens::HEADER],
    ) -> Option<(usize, usize)> {
        self.certificate =
            self.changes
                .sequence(manager)
                .map(|(epoch, sequence)| TokenCertificate {
                    epoch,
                    sequence,
                    manager,
                    header: *header,
                });
        let tokens = &self.store.tokens;
        if self.certificate.is_some() && tokens.certificate == self.certificate {
            return Some((tokens.rows.len(), tokens.bytes.len()));
        }
        None
    }

    fn carry(&mut self, rows: &mut [advisor_tokens::Row], bytes: &mut [u8]) {
        let tokens = &self.store.tokens;
        rows.copy_from_slice(&tokens.rows);
        bytes.copy_from_slice(&tokens.bytes);
        self.store.token_counts.reused += 1;
        self.store.token_counts.reused_rows += rows.len() as u64;
        self.carried = true;
    }

    fn read(&mut self, rows: &[advisor_tokens::Row], bytes: &[u8]) {
        let store = &mut *self.store;
        store.token_counts.read += 1;
        store.token_counts.read_rows += rows.len() as u64;
        let tokens = &mut store.tokens;
        tokens.certificate = None;
        let held = self.certificate.is_some()
            && allocation::advisory(|| {
                tokens.rows.clear();
                allocation::reserve(&mut tokens.rows, rows.len())?;
                tokens.rows.extend_from_slice(rows);
                tokens.bytes.clear();
                allocation::reserve(&mut tokens.bytes, bytes.len())?;
                tokens.bytes.extend_from_slice(bytes);
                std::io::Result::Ok(())
            })
            .is_ok();
        if held {
            tokens.certificate = self.certificate;
        }
        self.read = Some(held);
    }
}
