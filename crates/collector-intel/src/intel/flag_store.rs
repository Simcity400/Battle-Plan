//! ScriptFlags rows held between hours per flag manager
//! ([`crate::flag_changes`]).
//!
//! The walk still reads every scope's identity and manager root each hour. A
//! scope's held rows are carried, without reading its manager's header or
//! rows, only when all of these hold at this boundary: the same scope kind,
//! id and manager address as the held read, and no hooked writer touched the
//! manager since that read registered it (same watch epoch and sequence; a
//! load, gap, recovery, table change, poison or running writer revokes it).
//! Every writer of a manager's header and rows is a hooked site
//! ([`engine_mutations::script_flag_mutations`]), so the fresh read would return the
//! held rows: each manager's rows are sorted by token, so the carried copy
//! is byte-identical. Each hour also reads the carryable managers of its
//! sampled slice ([`crate::sampled_verify`], keyed by [`key`]; every one
//! under `flag_verify`) afresh, publishes the fresh rows and poisons reuse
//! on a difference; [`FlagReuse::plan`] takes that choice per scope.
//!
//! A quiet hour walks no scope at all. The latest complete walk that held every scope under its own
//! certificate keeps its rows in walk order, its countries, its scopes in walk order with each
//! scope's manager and row count, and the key taken before its first registration: the flag
//! domain's structure token ([`Changes::structure`]), the character roster's ([`Changes::roster`])
//! and the scope `(base, game state)`, with the epoch of the hour's take of the marked managers
//! ([`Changes::take`]). Every native writer of a scope root also builds, destroys or clears a
//! manager in the Same call  (game-state construction and its destructor game-state destruction for
//! the global root (`cgame_state::GLOBAL_FLAGS`); country construction, whose flag manager
//! (`ccountry::FLAG_MANAGER`) is never replaced, and dynamic reuse, whose id change in
//! dynamic-country allocation is followed by flag clearing through country appearance reset in the
//! same call; the state table, built by game-state reset and rebuilt only by the state editor
//! reload, through state construction and destroy state, the only writers of a state's vtable, id
//! the state id and its flag root), and each of those is a structure site, so an unchanged
//! structure token proves every global, country and state scope and its manager unchanged; an
//! unchanged roster token proves every character scope unchanged. Every other writer changes only
//! its manager's rows and marks it. When both tokens and the country census (the callback's
//! authenticated census, which costs no read) equal the held ones and the hour's take returned the
//! held walk's epoch, the walk publishes the held rows in walk order without walking any scope,
//! reading afresh only the managers the take marked, whose rows replace their held ones, and the
//! managers of its sampled slice, which it compares; it finds them through an index of the held
//! walk (by manager and by sampling bucket) and copies the held rows between them whole, touching
//! no other scope; a difference poisons the flag and roster domains and the hour walks afresh.
//! While the roster family burns in (`roster_verify`), no hour is quiet: each walk whose key
//! matched and whose take marked nothing is compared whole with the held rows.
//!
//! The store follows the IntelResidual job's loan and holds only the
//! managers of the latest complete walk, so its size follows live demand.
//! Allocation is advisory: a manager whose rows cannot be held is read
//! afresh next hour.
use super::ScopedFlag;
use crate::{
    flag_changes::{Changes, Counts},
    sampled_verify::{BURNED_IN_ROTATION, Check},
};
use engine_allocation as allocation;
use engine_native::{Memory, walk::Outcome};
use engine_scripts::{cflag_manager, cscript_flag};
use std::ops::Range;

/// What a quiet hour must find unchanged since the latest complete walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct QuietKey {
    structure: (u64, u64),
    roster: (u64, u64),
    scope: (usize, usize),
}

/// One scope of a walk, in walk order: its kind, id, manager and row count.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Visited {
    kind: u8,
    id: u32,
    manager: usize,
    rows: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Certificate {
    epoch: u64,
    sequence: u64,
    manager: usize,
    kind: u8,
    id: u32,
}

#[derive(Clone, Debug)]
struct Entry {
    certificate: Certificate,
    rows: Range<usize>,
}

/// One walk's held rows.
#[derive(Default)]
struct Held {
    /// Sorted by manager once the walk finishes.
    entries: Vec<Entry>,
    rows: Vec<cscript_flag::Row>,
}

impl Held {
    fn clear(&mut self) {
        self.entries.clear();
        self.rows.clear();
    }

    /// The held rows of exactly `certificate`.
    fn find(&self, certificate: &Certificate) -> Option<Range<usize>> {
        let index = self
            .entries
            .binary_search_by_key(&certificate.manager, |entry| entry.certificate.manager)
            .ok()?;
        let entry = &self.entries[index];
        (entry.certificate == *certificate).then(|| entry.rows.clone())
    }

    /// Holds `rows` for `certificate`; `false` when the storage could not
    /// grow, which only withholds the certificate.
    fn hold(&mut self, certificate: Certificate, rows: &[cscript_flag::Row]) -> bool {
        let stored = allocation::advisory(|| {
            allocation::reserve(&mut self.entries, 1)?;
            allocation::reserve(&mut self.rows, rows.len())?;
            std::io::Result::Ok(())
        });
        if stored.is_err() {
            return false;
        }
        let first = self.rows.len();
        self.rows.extend_from_slice(rows);
        self.entries.push(Entry {
            certificate,
            rows: first..self.rows.len(),
        });
        true
    }

    fn measure(&self) -> usize {
        self.entries.capacity() * size_of::<Entry>()
            + self.rows.capacity() * size_of::<cscript_flag::Row>()
    }
}

/// Rows of the latest complete walk and the walk being made.
#[derive(Default)]
pub struct FlagStore {
    current: Held,
    previous: Held,
    counts: Counts,
    scopes: usize,
    /// Allocated bytes as of the last finish, for the game thread.
    bytes: usize,
    /// Set on the game thread; the next walk drops every held row.
    forgotten: bool,
    /// Every scope of the walk being made has a held certificate.
    held_all: bool,
    /// Countries the walk being made visited, in walk order.
    walking: Vec<(usize, u32)>,
    /// What lets a quiet hour publish `published`: the key taken before the
    /// latest complete walk that held every scope.
    quiet: Option<QuietKey>,
    /// That walk's rows in walk order, its countries, its scopes in walk
    /// order and the epoch of the take before it.
    published: Vec<ScopedFlag>,
    countries: Vec<(usize, u32)>,
    order: Vec<Visited>,
    epoch: Option<u64>,
    /// The scopes the walk being made visited, in walk order.
    visiting: Vec<Visited>,
    /// The hour's take: the marked managers, sorted, and its epoch.
    marked: Vec<usize>,
    taken: Option<u64>,
    /// A quiet hour's fresh read of a marked or sampled manager.
    scratch: Vec<cscript_flag::Row>,
    /// The index of `order` a quiet hour visits by ([`Index`]).
    index: Index,
}

/// Where a quiet hour finds the scopes it reads, so it touches only those
/// and copies the held rows between them whole: each scope's first row in
/// `published`, the scopes by sampling bucket and by manager.
#[derive(Default)]
struct Index {
    /// `order`'s first published row per scope, and the row count after
    /// the last.
    starts: Vec<usize>,
    /// Scopes in walk order per bucket of [`key`] modulo
    /// [`BURNED_IN_ROTATION`], from `bucket_starts`.
    buckets: Vec<u32>,
    bucket_starts: Vec<u32>,
    /// `(manager, scope)`, sorted: a quiet walk's managers are distinct.
    managers: Vec<(usize, u32)>,
    /// Each bucket's next free place while the buckets are filled.
    cursor: Vec<u32>,
    /// The hour's scopes to read, in walk order.
    visits: Vec<u32>,
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// The scopes this thread's latest quiet hour visited in its publishing
    /// loop ([`FlagStore::visited_scopes`]).
    static EXAMINED: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Buckets of the sampling key: whether any [`Check`] covers a key depends
/// only on the key modulo the longest rotation, which the daily one divides.
const BUCKETS: usize = BURNED_IN_ROTATION as usize;
const _: () = assert!(BURNED_IN_ROTATION.is_multiple_of(crate::sampled_verify::ROTATION));

impl Index {
    fn clear(&mut self) {
        self.starts.clear();
        self.buckets.clear();
        self.bucket_starts.clear();
        self.managers.clear();
    }

    /// Indexes the scopes `order` of a held walk; `false` when the storage
    /// could not grow.
    fn build(&mut self, order: &[Visited]) -> bool {
        self.clear();
        let len = order.len();
        let stored = allocation::advisory(|| {
            allocation::reserve(&mut self.starts, len + 1)?;
            allocation::reserve(&mut self.buckets, len)?;
            allocation::reserve(&mut self.bucket_starts, BUCKETS + 1)?;
            allocation::reserve(&mut self.cursor, BUCKETS + 1)?;
            allocation::reserve(&mut self.managers, len)
        });
        if stored.is_err() || u32::try_from(len).is_err() {
            self.clear();
            return false;
        }
        self.rows(order);
        self.bucket_starts.resize(BUCKETS + 1, 0);
        for visited in order {
            self.bucket_starts[bucket(visited) + 1] += 1;
        }
        for at in 0..BUCKETS {
            self.bucket_starts[at + 1] += self.bucket_starts[at];
        }
        // Within the reservations above: nothing here allocates.
        self.cursor.clear();
        self.cursor.extend_from_slice(&self.bucket_starts);
        self.buckets.resize(len, 0);
        for (scope, visited) in order.iter().enumerate() {
            let slot = &mut self.cursor[bucket(visited)];
            self.buckets[*slot as usize] = scope as u32;
            *slot += 1;
        }
        self.managers.extend(
            order
                .iter()
                .enumerate()
                .map(|(scope, visited)| (visited.manager, scope as u32)),
        );
        self.managers.sort_unstable();
        true
    }

    /// Each scope's first published row, after its rows were renewed.
    fn rows(&mut self, order: &[Visited]) {
        self.starts.clear();
        let mut at = 0;
        self.starts.push(at);
        for visited in order {
            at += visited.rows;
            self.starts.push(at);
        }
    }

    /// The scopes a quiet hour reads: each marked manager's and each one
    /// `check` covers, in walk order; `false` when they cannot be listed.
    fn plan(&mut self, marked: &[usize], check: Check) -> bool {
        self.visits.clear();
        let covered: usize = (0..BUCKETS)
            .filter(|&at| check.covers(at as u64))
            .map(|at| (self.bucket_starts[at + 1] - self.bucket_starts[at]) as usize)
            .sum();
        if allocation::advisory(|| allocation::reserve(&mut self.visits, marked.len() + covered))
            .is_err()
        {
            return false;
        }
        for &manager in marked {
            if let Ok(at) = self
                .managers
                .binary_search_by_key(&manager, |&(manager, _)| manager)
            {
                self.visits.push(self.managers[at].1);
            }
        }
        for at in (0..BUCKETS).filter(|&at| check.covers(at as u64)) {
            let range = self.bucket_starts[at] as usize..self.bucket_starts[at + 1] as usize;
            self.visits.extend_from_slice(&self.buckets[range]);
        }
        self.visits.sort_unstable();
        self.visits.dedup();
        true
    }

    fn measure(&self) -> usize {
        self.starts.capacity() * size_of::<usize>()
            + (self.buckets.capacity()
                + self.bucket_starts.capacity()
                + self.cursor.capacity()
                + self.visits.capacity())
                * size_of::<u32>()
            + self.managers.capacity() * size_of::<(usize, u32)>()
    }
}

/// A scope's sampling bucket.
fn bucket(visited: &Visited) -> usize {
    (key(visited.kind, visited.id) % BURNED_IN_ROTATION) as usize
}

impl FlagStore {
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

    /// The scopes this thread's latest quiet hour visited in its publishing
    /// loop, as walk-order indices.
    #[cfg(any(test, feature = "test-support"))]
    pub fn visited_scopes() -> Vec<u32> {
        EXAMINED.with(|examined| examined.borrow().clone())
    }

    fn begin(&mut self) {
        if std::mem::take(&mut self.forgotten) {
            self.current.clear();
            self.previous.clear();
            self.quiet = None;
        }
        std::mem::swap(&mut self.current, &mut self.previous);
        self.current.clear();
        self.counts = Counts::default();
        self.scopes = 0;
        self.held_all = true;
        self.walking.clear();
        self.visiting.clear();
    }

    /// The hour's take of the marked managers, before anything else of the
    /// hour; `taken` is `None` when it certified nothing.
    fn take(&mut self, changes: &dyn Changes) {
        self.marked.clear();
        let marked = &mut self.marked;
        let mut complete = true;
        let taken = changes.take(&mut |manager| {
            if allocation::advisory(|| allocation::reserve(marked, 1)).is_ok() {
                marked.push(manager);
            } else {
                complete = false;
            }
        });
        self.marked.sort_unstable();
        self.marked.dedup();
        self.taken = taken.filter(|_| complete);
    }

    /// Keeps this walk's rows only when every scope was read or carried
    /// completely, and publishes its counts. `key` was taken before the
    /// walk; `rows` are its rows in walk order.
    fn finish(&mut self, complete: bool, key: Option<QuietKey>, rows: &[ScopedFlag]) {
        if complete {
            let entries = &mut self.current.entries;
            entries.sort_unstable_by_key(|entry| entry.certificate.manager);
            // A manager occurring twice names no single held read.
            let mut index = 0;
            while index < entries.len() {
                let manager = entries[index].certificate.manager;
                let end = entries[index..]
                    .iter()
                    .position(|entry| entry.certificate.manager != manager)
                    .map_or(entries.len(), |offset| index + offset);
                if end - index > 1 {
                    entries.drain(index..end);
                    self.held_all = false;
                } else {
                    index = end;
                }
            }
        } else {
            self.current.clear();
        }
        self.previous.clear();
        // Every scope held under one certificate, with no mismatch: a later
        // hour with this key publishes these rows.
        self.quiet = key.filter(|_| {
            complete
                && self.held_all
                && self.counts.mismatches == 0
                && self.current.entries.len() == self.scopes
                && self.visiting.len() == self.scopes
                && self.taken.is_some()
        });
        let held = self.quiet.is_some()
            && allocation::advisory(|| {
                self.published.clear();
                self.countries.clear();
                allocation::reserve_exact(&mut self.published, rows.len())?;
                allocation::reserve_exact(&mut self.countries, self.walking.len())
            })
            .is_ok();
        if held {
            self.published.extend_from_slice(rows);
            self.countries.extend_from_slice(&self.walking);
            std::mem::swap(&mut self.order, &mut self.visiting);
            self.epoch = self.taken;
        }
        // Only an indexed walk lets a later quiet hour publish its rows.
        let indexed = held && self.index.build(&self.order);
        if !indexed {
            self.index.clear();
            self.quiet = None;
            self.published.clear();
            self.countries.clear();
            self.order.clear();
            self.epoch = None;
        }
        self.measure();
        crate::flag_changes::record(&self.counts, complete.then_some(self.scopes));
    }

    fn measure(&mut self) {
        self.bytes = self.current.measure()
            + self.previous.measure()
            + self.walking.capacity() * size_of::<(usize, u32)>()
            + self.countries.capacity() * size_of::<(usize, u32)>()
            + self.published.capacity() * size_of::<ScopedFlag>()
            + (self.order.capacity() + self.visiting.capacity()) * size_of::<Visited>()
            + self.marked.capacity() * size_of::<usize>()
            + self.scratch.capacity() * size_of::<cscript_flag::Row>()
            + self.index.measure();
    }

    /// A quiet hour's rows into `output`: the held rows in walk order, each
    /// marked manager's and each manager `check` covers read afresh in its
    /// place. A covered manager no take marked is compared with its held
    /// rows. Only those scopes are visited, in walk order ([`Index::plan`]);
    /// the held rows between them are copied whole. `None` when a read did
    /// not complete or `output` has no room.
    fn publish(
        &mut self,
        memory: &impl Memory,
        base: usize,
        check: Check,
        output: &mut engine_native::walk::Buffer<'_, ScopedFlag>,
    ) -> Option<Sample> {
        let mut sample = Sample::default();
        let Self {
            published,
            order,
            marked,
            scratch,
            index,
            ..
        } = self;
        if index.starts.len() != order.len() + 1 || !index.plan(marked, check) {
            return None;
        }
        let mut len = 0usize;
        // The held rows of scopes `from..to`, copied whole after `len`.
        let copy = |output: &mut engine_native::walk::Buffer<'_, ScopedFlag>,
                    len: &mut usize,
                    from: usize,
                    to: usize|
         -> Option<()> {
            let held = published.get(index.starts[from]..index.starts[to])?;
            let end = len.checked_add(held.len())?;
            if output.is_growing() {
                output.ensure(end).ok()?;
            } else if output.len() < end {
                return None;
            }
            output[*len..end].copy_from_slice(held);
            *len = end;
            Some(())
        };
        let mut cursor = 0usize;
        #[cfg(any(test, feature = "test-support"))]
        EXAMINED.with(|examined| examined.borrow_mut().clear());
        for &scope in &index.visits {
            #[cfg(any(test, feature = "test-support"))]
            EXAMINED.with(|examined| examined.borrow_mut().push(scope));
            let at = scope as usize;
            copy(output, &mut len, cursor, at)?;
            let visited = &mut order[at];
            let held = published.get(index.starts[at]..index.starts[at + 1])?;
            let fresh = marked.binary_search(&visited.manager).is_ok();
            let compare = !fresh && check.covers(key(visited.kind, visited.id));
            let head = cflag_manager::read_header(memory, visited.manager);
            let read = cflag_manager::walk_header_buffer(
                memory,
                base,
                head,
                engine_native::walk::Buffer::Growing(scratch),
            );
            let Outcome::Rows { len: count } = read else {
                return None;
            };
            sample.read += 1;
            sample.rows += count as u64;
            if compare {
                sample.verified += 1;
                if held
                    .iter()
                    .map(|flag| &flag.flag)
                    .ne(scratch[..count].iter())
                {
                    sample.differs = true;
                    return Some(sample);
                }
            }
            visited.rows = count;
            let end = len.checked_add(count)?;
            if output.is_growing() {
                output.ensure(end).ok()?;
            } else if output.len() < end {
                return None;
            }
            for (target, flag) in output[len..end].iter_mut().zip(&scratch[..count]) {
                *target = ScopedFlag {
                    scope_kind: visited.kind,
                    scope_id: visited.id,
                    flag: *flag,
                };
            }
            sample.renewed |= fresh && output[len..end] != *held;
            len = end;
            cursor = at + 1;
        }
        copy(output, &mut len, cursor, order.len())?;
        sample.len = len;
        Some(sample)
    }
}

/// One visited scope of the walk being made, in walk order; a scope that
/// cannot be listed withholds the next quiet hour.
fn visit(visiting: &mut Vec<Visited>, held_all: &mut bool, certificate: &Certificate, rows: usize) {
    if allocation::advisory(|| allocation::reserve(visiting, 1)).is_ok() {
        visiting.push(Visited {
            kind: certificate.kind,
            id: certificate.id,
            manager: certificate.manager,
            rows,
        });
    } else {
        *held_all = false;
    }
}

/// What a quiet hour's reads found.
#[derive(Clone, Copy, Debug, Default)]
struct Sample {
    /// Managers read afresh (marked or sampled), and their rows.
    read: u64,
    rows: u64,
    /// Sampled managers no take marked, compared with their held rows.
    verified: u64,
    differs: bool,
    /// A marked manager's rows differ from its held ones.
    renewed: bool,
    /// Rows published.
    len: usize,
}

/// One walk's view of the store and the observation that certifies reuse.
pub struct FlagReuse<'a> {
    pub store: &'a mut FlagStore,
    pub changes: &'a dyn Changes,
    /// Which carryable scopes this hour reads afresh and compares.
    pub check: Check,
    /// No hour is quiet: a walk whose key matched is compared whole with the
    /// held rows instead (the roster family's burn-in, `roster_verify`).
    pub quiet_verify: bool,
    /// Taken before the walk's first registration or read.
    key: Option<QuietKey>,
    /// The key matched the held one, so the walk is compared with it.
    compare: bool,
    roster: crate::roster_changes::Counts,
}

/// A scope's stable sampling key: its kind and id.
pub fn key(kind: u8, id: u32) -> u64 {
    (u64::from(kind) << 32) | u64::from(id)
}

/// One scope's plan: carry its held rows, or read it afresh.
#[derive(Clone, Debug)]
pub struct Plan {
    certificate: Option<Certificate>,
    /// The carryable held rows, indexing the previous walk's rows.
    held: Option<Range<usize>>,
    /// Whether the held rows may be carried without a fresh read.
    carry: bool,
}

impl Plan {
    /// Rows the caller must have room for to carry the scope, when it may.
    pub fn carry_rows(&self) -> Option<usize> {
        self.held.as_ref().filter(|_| self.carry).map(Range::len)
    }
}

impl<'a> FlagReuse<'a> {
    /// The walk begins the store at [`Self::quiet_hour`].
    pub fn begin(store: &'a mut FlagStore, changes: &'a dyn Changes, check: Check) -> Self {
        Self {
            store,
            changes,
            check,
            quiet_verify: crate::roster_changes::VERIFY,
            key: None,
            compare: false,
            roster: Default::default(),
        }
    }

    /// Tests: whether a matching key publishes the held rows.
    #[cfg(any(test, feature = "test-support"))]
    pub fn verifying_quiet(mut self, verify: bool) -> Self {
        self.quiet_verify = verify;
        self
    }

    /// Before the walk: takes the key and, when the hour is quiet, publishes
    /// the held rows into `output` and returns their outcome; otherwise
    /// begins the store for a full walk. `countries` says whether this hour's
    /// countries, in walk order, are [`Self::held_countries`]; with the
    /// callback's authenticated census it reads nothing.
    pub(super) fn quiet_hour(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        output: &mut engine_native::walk::Buffer<'_, ScopedFlag>,
        countries: impl FnOnce(&[(usize, u32)]) -> bool,
    ) -> Option<Outcome> {
        let structure = self.changes.structure();
        let roster = self.changes.roster();
        self.key = structure.zip(roster).map(|(structure, roster)| QuietKey {
            structure,
            roster,
            scope: (base, state),
        });
        let store = &mut *self.store;
        store.take(self.changes);
        let matched = !store.forgotten
            && self.key.is_some()
            && store.quiet == self.key
            && store.taken.is_some()
            && store.taken == store.epoch
            && self.check != Check::Every
            && countries(&store.countries);
        if matched && self.quiet_verify {
            // A marked manager's held rows are stale: only an unmarked hour
            // compares the walk with them.
            self.compare = store.marked.is_empty();
        } else if matched {
            match store.publish(memory, base, self.check, output) {
                Some(sample) if !sample.differs => {
                    let len = sample.len;
                    // A marked manager that changed renews the held rows; a
                    // copy that cannot grow withholds the next quiet hour.
                    if sample.renewed
                        && allocation::advisory(|| {
                            store.published.clear();
                            allocation::reserve_exact(&mut store.published, len)
                        })
                        .is_err()
                    {
                        store.quiet = None;
                    } else if sample.renewed {
                        store.published.extend_from_slice(&output[..len]);
                        store.index.rows(&store.order);
                    }
                    let held_rows = store.current.rows.len() as u64;
                    let scopes = store.order.len() as u64;
                    store.counts = Counts {
                        reused: scopes - sample.read,
                        read: sample.read,
                        verified: sample.verified,
                        mismatches: 0,
                        reused_rows: held_rows.saturating_sub(sample.rows),
                        read_rows: sample.rows,
                        quiet: true,
                    };
                    store.measure();
                    crate::flag_changes::record(&store.counts, Some(store.scopes));
                    crate::roster_changes::record(
                        crate::roster_changes::Table::ScriptFlags,
                        &crate::roster_changes::Counts {
                            quiet: true,
                            ..Default::default()
                        },
                    );
                    return Some(Outcome::Rows { len });
                }
                Some(sample) => {
                    // A missed write: nothing certifies reuse any more, and
                    // this hour walks every scope afresh.
                    self.changes.scope_mismatch();
                    store.begin();
                    store.counts.verified = sample.verified;
                    store.counts.mismatches = 1;
                    self.roster = crate::roster_changes::Counts {
                        read: true,
                        verified: 1,
                        mismatches: 1,
                        ..Default::default()
                    };
                    return None;
                }
                None => {}
            }
        }
        store.begin();
        self.roster.read = true;
        None
    }

    /// One country scope of the walk, in walk order.
    pub fn country(&mut self, pointer: usize, id: u32) {
        let store = &mut *self.store;
        if allocation::advisory(|| allocation::reserve(&mut store.walking, 1)).is_ok() {
            store.walking.push((pointer, id));
        } else {
            store.held_all = false;
        }
    }

    /// Registers one scope's manager and plans its rows: carryable when the
    /// held read of the same scope and manager is proved current, and
    /// carried without a read unless `verify` (the hour's [`Check`] covers
    /// it). Registration precedes every read it certifies.
    pub fn plan(&mut self, verify: bool, kind: u8, id: u32, manager: usize) -> Plan {
        self.store.scopes += 1;
        let certificate = self
            .changes
            .sequence(manager)
            .map(|(epoch, sequence)| Certificate {
                epoch,
                sequence,
                manager,
                kind,
                id,
            });
        self.store.held_all &= certificate.is_some();
        let held = certificate.and_then(|certificate| self.store.previous.find(&certificate));
        Plan {
            certificate,
            carry: held.is_some() && !verify,
            held,
        }
    }

    /// The carried rows of a plan whose [`Plan::carry_rows`] the caller has
    /// room for, held again for the next hour.
    pub fn carry(&mut self, plan: &Plan) -> &[cscript_flag::Row] {
        let (Some(certificate), Some(range)) = (plan.certificate, plan.held.clone()) else {
            return &[];
        };
        let FlagStore {
            previous,
            current,
            counts,
            held_all,
            visiting,
            ..
        } = &mut *self.store;
        let rows = &previous.rows[range];
        counts.reused += 1;
        counts.reused_rows += rows.len() as u64;
        // A failed hold only withholds the next hour's certificate.
        *held_all &= current.hold(certificate, rows);
        visit(visiting, held_all, &certificate, rows.len());
        rows
    }

    /// A fresh read of a planned scope: `rows` are its rows sorted by token
    /// when `outcome` is complete. They are compared with carryable held
    /// rows (a difference poisons reuse) and held for the next hour.
    pub fn read(&mut self, plan: &Plan, outcome: Outcome, rows: &[cscript_flag::Row]) {
        let store = &mut *self.store;
        store.counts.read += 1;
        store.counts.read_rows += rows.len() as u64;
        if let Some(range) = plan.held.clone() {
            store.counts.verified += 1;
            if !matches!(outcome, Outcome::Rows { .. }) || store.previous.rows[range] != *rows {
                store.counts.mismatches += 1;
                self.changes.mismatch();
            }
        }
        if let (Some(certificate), Outcome::Rows { .. }) = (plan.certificate, outcome) {
            if !store.current.hold(certificate, rows) {
                store.held_all = false;
            }
            visit(
                &mut store.visiting,
                &mut store.held_all,
                &certificate,
                rows.len(),
            );
        }
    }

    /// Ends the walk: `complete` when every scope was read or carried whole;
    /// `rows` are the walk's rows in walk order. A walk whose key matched the
    /// held one (verify mode) is compared whole with the held rows first.
    pub(super) fn finish(mut self, complete: bool, rows: &[ScopedFlag]) {
        if self.compare && complete {
            let store = &*self.store;
            let same = store.published == rows && store.countries == store.walking;
            self.roster.verified += 1;
            if !same {
                self.roster.mismatches += 1;
                self.store.counts.mismatches += 1;
                self.changes.scope_mismatch();
            }
        }
        crate::roster_changes::record(crate::roster_changes::Table::ScriptFlags, &self.roster);
        let key = self.key.filter(|_| self.check != Check::Every);
        self.store.finish(complete, key, rows);
    }
}
