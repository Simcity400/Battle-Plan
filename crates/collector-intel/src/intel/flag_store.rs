use super::ScopedFlag;
use crate::flag_changes::{Changes, Counts};
use engine_allocation as allocation;
use engine_native_memory::{Memory, walk::Outcome};
use engine_scripts::{cflag_manager, cscript_flag};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct QuietKey {
    structure: (u64, u64),
    roster: (u64, u64),
    scope: (usize, usize),
}

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

#[derive(Default)]
struct Held {
    entries: Vec<Entry>,
    rows: Vec<cscript_flag::Row>,
}

impl Held {
    fn clear(&mut self) {
        self.entries.clear();
        self.rows.clear();
    }

    fn find(&self, certificate: &Certificate) -> Option<Range<usize>> {
        let index = self
            .entries
            .binary_search_by_key(&certificate.manager, |entry| entry.certificate.manager)
            .ok()?;
        let entry = &self.entries[index];
        (entry.certificate == *certificate).then(|| entry.rows.clone())
    }

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

#[derive(Default)]
pub struct FlagStore {
    current: Held,
    previous: Held,
    counts: Counts,
    scopes: usize,
    bytes: usize,
    forgotten: bool,
    held_all: bool,
    walking: Vec<(usize, u32)>,
    quiet: Option<QuietKey>,
    published: Vec<ScopedFlag>,
    countries: Vec<(usize, u32)>,
    order: Vec<Visited>,
    epoch: Option<u64>,
    visiting: Vec<Visited>,
    marked: Vec<usize>,
    taken: Option<u64>,
    scratch: Vec<cscript_flag::Row>,
    index: Index,
    generation: u64,
    witnessed: Option<u64>,
}

#[derive(Default)]
struct Index {
    starts: Vec<usize>,
    managers: Vec<(usize, u32)>,
    visits: Vec<u32>,
}

#[cfg(any(test, feature = "test-support"))]
thread_local! {
    static EXAMINED: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
}

impl Index {
    fn clear(&mut self) {
        self.starts.clear();
        self.managers.clear();
    }

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

    fn rows(&mut self, order: &[Visited]) {
        self.starts.clear();
        let mut at = 0;
        self.starts.push(at);
        for visited in order {
            at += visited.rows;
            self.starts.push(at);
        }
    }

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

    pub fn forget(&mut self) {
        self.forgotten = true;
    }

    pub fn allocated_bytes(&self) -> usize {
        self.bytes
    }

    pub fn witnessed(&self) -> Option<u64> {
        self.witnessed
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn counts(&self) -> Counts {
        self.counts
    }

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

    fn finish(&mut self, complete: bool, key: Option<QuietKey>, rows: &[ScopedFlag]) {
        if complete {
            let entries = &mut self.current.entries;
            entries.sort_unstable_by_key(|entry| entry.certificate.manager);
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

    fn publish(
        &mut self,
        memory: &impl Memory,
        base: usize,
        output: &mut engine_native_memory::walk::Buffer<'_, ScopedFlag>,
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
        let copy = |output: &mut engine_native_memory::walk::Buffer<'_, ScopedFlag>,
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
                engine_native_memory::walk::Buffer::Growing(scratch),
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

#[derive(Clone, Copy, Debug, Default)]
struct Sample {
    read: u64,
    rows: u64,
    renewed: bool,
    len: usize,
}

pub struct FlagReuse<'a> {
    pub store: &'a mut FlagStore,
    pub changes: &'a dyn Changes,
    key: Option<QuietKey>,
    roster: crate::roster_changes::Counts,
}

#[derive(Clone, Debug)]
pub struct Plan {
    certificate: Option<Certificate>,
    held: Option<Range<usize>>,
}

impl Plan {
    pub fn carry_rows(&self) -> Option<usize> {
        self.held.as_ref().map(Range::len)
    }
}

impl<'a> FlagReuse<'a> {
    pub fn begin(store: &'a mut FlagStore, changes: &'a dyn Changes) -> Self {
        Self {
            store,
            changes,
            key: None,
            roster: Default::default(),
        }
    }

    pub(super) fn quiet_hour(
        &mut self,
        memory: &impl Memory,
        base: usize,
        state: usize,
        output: &mut engine_native_memory::walk::Buffer<'_, ScopedFlag>,
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

    pub fn country(&mut self, pointer: usize, id: u32) {
        let store = &mut *self.store;
        if allocation::advisory(|| allocation::reserve(&mut store.walking, 1)).is_ok() {
            store.walking.push((pointer, id));
        } else {
            store.held_all = false;
        }
    }

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
        *held_all &= current.hold(certificate, rows);
        visit(visiting, held_all, &certificate, rows.len());
        rows
    }

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

    pub(super) fn finish(self, complete: bool, rows: &[ScopedFlag]) {
        crate::roster_changes::record(crate::roster_changes::Table::ScriptFlags, &self.roster);
        self.store.finish(complete, self.key, rows);
    }
}
