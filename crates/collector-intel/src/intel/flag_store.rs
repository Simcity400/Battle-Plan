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
//! is byte-identical.
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
//! reading afresh only the managers the take marked, whose rows replace their held ones; it finds
//! them through an index of the held walk by manager and copies the held rows between them whole,
//! touching no other scope.
//!
//! The store follows the IntelResidual job's loan and holds only the
//! managers of the latest complete walk, so its size follows live demand.
//! Allocation is advisory: a manager whose rows cannot be held is read
//! afresh next hour.
use super::ScopedFlag;
use crate::flag_changes::{Changes, Counts};
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
    /// A quiet hour's fresh read of a marked manager.
    scratch: Vec<cscript_flag::Row>,
    /// The index of `order` a quiet hour visits by ([`Index`]).
    index: Index,
    /// Names `published` ([`collector_projection::projection::generation`]).
    generation: u64,
    /// `published`'s generation when the latest walk's rows are exactly it.
    witnessed: Option<u64>,
}

/// Where a quiet hour finds the scopes it reads, so it touches only those
/// and copies the held rows between them whole: each scope's first row in
/// `published` and the scopes by manager.
#[derive(Default)]
struct Index {
    /// `order`'s first published row per scope, and the row count after
    /// the last.
    starts: Vec<usize>,
    /// `(manager, scope)`, sorted: a quiet walk's managers are distinct.
    managers: Vec<(usize, u32)>,
    /// The hour's scopes to read, in walk order.
    visits: Vec<u32>,
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    /// The scopes this thread's latest quiet hour visited in its publishing
    /// loop ([`FlagStore::visited_scopes`]).
    static EXAMINED: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
}

impl Index {
    fn clear(&mut self) {
        self.starts.clear();
        self.managers.clear();
    }

    /// Indexes the scopes `order` of a held walk; `false` when the storage
    /// could not grow.
    fn build(&mut self, order: &[Visited]) -> bool {
        self.clear();
        let len = order.len();
        let stored = allocation::advisory(|| {
            allocation::reserve(&mut self.starts, len + 1)?;
            allocation::reserve(&mut self.managers, len)
        });
        if stored.is_err() || u32::try_from(len).is_err() {
            self.clear();
            return false;
        }
        // Within the reservations above: nothing here allocates.
        self.rows(order);
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

    /// The scopes a quiet hour reads: each marked manager's, in walk order;
    /// `false` when they cannot be listed.
    fn plan(&mut self, marked: &[usize]) -> bool {
        self.visits.clear();
        if allocation::advisory(|| allocation::reserve(&mut self.visits, marked.len()))
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
        self.visits.sort_unstable();
        self.visits.dedup();
        true
    }

    fn measure(&self) -> usize {
        self.starts.capacity() * size_of::<usize>()
            + self.visits.capacity() * size_of::<u32>()
            + self.managers.capacity() * size_of::<(usize, u32)>()
    }
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

    /// The generation of the held rows the latest walk published exactly.
    pub fn witnessed(&self) -> Option<u64> {
        self.witnessed
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
        // Every scope held under one certificate: a later hour with this key
        // publishes these rows.
        self.quiet = key.filter(|_| {
            complete
                && self.held_all
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
        if indexed {
            self.generation = collector_projection::projection::generation();
            self.witnessed = Some(self.generation);
        } else {
            self.index.clear();
            self.quiet = None;
            self.published.clear();
            self.countries.clear();
            self.order.clear();
            self.epoch = None;
            self.witnessed = None;
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
    /// marked manager's read afresh in its place. Only those scopes are
    /// visited, in walk order ([`Index::plan`]); the held rows between them
    /// are copied whole. `None` when a read did not complete or `output` has
    /// no room.
    fn publish(
        &mut self,
        memory: &impl Memory,
        base: usize,
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
        if index.starts.len() != order.len() + 1 || !index.plan(marked) {
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
            sample.renewed |= output[len..end] != *held;
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
    /// Managers read afresh, and their rows.
    read: u64,
    rows: u64,
    /// A marked manager's rows differ from its held ones.
    renewed: bool,
    /// Rows published.
    len: usize,
}

/// One walk's view of the store and the observation that certifies reuse.
pub struct FlagReuse<'a> {
    pub store: &'a mut FlagStore,
    pub changes: &'a dyn Changes,
    /// Taken before the walk's first registration or read.
    key: Option<QuietKey>,
    roster: crate::roster_changes::Counts,
}

/// One scope's plan: carry its held rows, or read it afresh.
#[derive(Clone, Debug)]
pub struct Plan {
    certificate: Option<Certificate>,
    /// The carryable held rows, indexing the previous walk's rows.
    held: Option<Range<usize>>,
}

impl Plan {
    /// Rows the caller must have room for to carry the scope, when it may.
    pub fn carry_rows(&self) -> Option<usize> {
        self.held.as_ref().map(Range::len)
    }
}

impl<'a> FlagReuse<'a> {
    /// The walk begins the store at [`Self::quiet_hour`].
    pub fn begin(store: &'a mut FlagStore, changes: &'a dyn Changes) -> Self {
        Self {
            store,
            changes,
            key: None,
            roster: Default::default(),
        }
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
        store.witnessed = None;
        store.take(self.changes);
        let matched = !store.forgotten
            && self.key.is_some()
            && store.quiet == self.key
            && store.taken.is_some()
            && store.taken == store.epoch
            && countries(&store.countries);
        if matched && let Some(sample) = store.publish(memory, base, output) {
            let len = sample.len;
            // A marked manager that changed renews the held rows; a copy that
            // cannot grow withholds the next quiet hour.
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
                store.generation = collector_projection::projection::generation();
            }
            store.witnessed = store.quiet.is_some().then_some(store.generation);
            let held_rows = store.current.rows.len() as u64;
            let scopes = store.order.len() as u64;
            store.counts = Counts {
                reused: scopes - sample.read,
                read: sample.read,
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

    /// Registers one scope's manager and plans its rows: carried without a
    /// read when the held read of the same scope and manager is proved
    /// current. Registration precedes every read it certifies.
    pub fn plan(&mut self, kind: u8, id: u32, manager: usize) -> Plan {
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
        Plan { certificate, held }
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
    /// when `outcome` is complete. They are held for the next hour.
    pub fn read(&mut self, plan: &Plan, outcome: Outcome, rows: &[cscript_flag::Row]) {
        let store = &mut *self.store;
        store.counts.read += 1;
        store.counts.read_rows += rows.len() as u64;
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
    /// `rows` are the walk's rows in walk order.
    pub(super) fn finish(self, complete: bool, rows: &[ScopedFlag]) {
        crate::roster_changes::record(crate::roster_changes::Table::ScriptFlags, &self.roster);
        self.store.finish(complete, self.key, rows);
    }
}
