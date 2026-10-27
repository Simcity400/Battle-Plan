//! StateBuildings and ProvinceBuildings rows held between hours per building
//! status ([`crate::building_changes`]).
//!
//! A walk reads and authenticates every state's and province's identity and
//! status headers. It registers every status, with or without buildings,
//! before reading it. A status's held rows are carried, without reading its
//! buildings, only when all of these hold at this boundary: the same status
//! address and scope, the same gathered identity (flags, location id,
//! country) and building, index and active vector headers as the held read,
//! and no hooked writer touched the status since that read registered it
//! (same watch epoch and sequence; a load, gap, recovery, table change,
//! poison, an unnamed receiver or a running writer revokes it). Every writer
//! of a recorded field is a hooked site ([`engine_mutations::building_mutations`]), so
//! the fresh read would return the held rows. A status without buildings has
//! no rows to carry: it is still staged so that its active vector is checked
//! as every walk checks it.
//!
//! A settled hour carries both tables without walking the locations: the latest walk completed both
//! tables and held every status under a certificate, the hour's take of the domain's marks ([`Changes::take`]) certifies the epoch taken before that
//! walk, and the state and province table headers (`cgame_state::STATES`, `cgame_state::PROVINCES`)
//! read as they did then. The take visits every registered status a hooked writer touched since the
//! previous take, and every walked status is registered, so every other status is the one held. The
//! sites cover every writer of what locates a status as well as of its rows and headers, and a
//! status's identity is written only by its constructor (a load or reset changes the epoch, nudge
//! mode poisons), so every location and every untouched status and building is the one held. The
//! hour reads each touched status afresh from the status alone ([`cbuilding_status::refresh`],
//! which requires its identity to be the held one), registering it first, and holds its fresh rows
//! at its place in walk order; a quiet hour, whose take visited no held status, reads none. When a
//! touched status does not read, the hour walks every location afresh.
//!
//! The store follows the Governance job's loan and holds only the statuses
//! of the latest complete walk, so its size follows live demand. Allocation
//! is advisory: a status whose rows cannot be held is read afresh next hour,
//! and the next hour then walks.
use crate::building_changes::{Changes, Counts};
use engine_allocation as allocation;
use engine_native::{
    Memory, cbuilding, cbuilding_status, cbuilding_status::Carry, cbuilding_status::Location,
    cbuilding_status::Walk, walk::Outcome,
};
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Certificate {
    epoch: u64,
    sequence: u64,
    location: Location,
}

#[derive(Clone, Debug)]
struct Entry {
    certificate: Certificate,
    rows: Range<usize>,
    /// Its walk's plan token, which orders the walk.
    order: usize,
}

/// One walk's held statuses and rows, at their output positions: the state
/// table's rows, then the province table's.
#[derive(Default)]
struct Held {
    /// In walk order once the walk finishes.
    entries: Vec<Entry>,
    /// Indices of `entries` by status; a status held twice is left out.
    by_status: Vec<usize>,
    /// A status that is not held leaves its span unset.
    rows: Vec<cbuilding::Row>,
}

impl Held {
    fn clear(&mut self) {
        self.entries.clear();
        self.by_status.clear();
        self.rows.clear();
    }

    /// The entry holding `status`.
    fn index(&self, status: usize) -> Option<usize> {
        self.by_status
            .binary_search_by_key(&status, |&index| {
                self.entries[index].certificate.location.status
            })
            .ok()
            .map(|at| self.by_status[at])
    }

    /// The held rows of exactly `certificate`.
    fn find(&self, certificate: &Certificate) -> Option<Range<usize>> {
        let entry = &self.entries[self.index(certificate.location.status)?];
        (entry.certificate == *certificate).then(|| entry.rows.clone())
    }

    /// Holds `rows` for `certificate`, planned `order`th, at output position
    /// `at`; `false` when the storage could not grow, which only withholds
    /// the certificate.
    fn hold(
        &mut self,
        certificate: Certificate,
        order: usize,
        rows: &[cbuilding::Row],
        at: usize,
    ) -> bool {
        let Some(end) = at.checked_add(rows.len()) else {
            return false;
        };
        let missing = end.saturating_sub(self.rows.len());
        let stored = allocation::advisory(|| {
            allocation::reserve(&mut self.entries, 1)?;
            allocation::reserve(&mut self.rows, missing)?;
            std::io::Result::Ok(())
        });
        if stored.is_err() {
            return false;
        }
        if end > self.rows.len() {
            self.rows.resize(end, cbuilding::Row::default());
        }
        self.rows[at..end].copy_from_slice(rows);
        self.entries.push(Entry {
            certificate,
            rows: at..end,
            order,
        });
        true
    }

    /// Puts the entries in walk order and indexes them by status; `false`
    /// when a status occurs twice, naming no single held read, or the index
    /// could not grow.
    fn order(&mut self) -> bool {
        let Self {
            entries, by_status, ..
        } = self;
        entries.sort_unstable_by_key(|entry| entry.order);
        by_status.clear();
        if allocation::advisory(|| allocation::reserve(by_status, entries.len())).is_err() {
            return false;
        }
        by_status.extend(0..entries.len());
        let status = |index: usize| entries[index].certificate.location.status;
        by_status.sort_unstable_by_key(|&index| status(index));
        let mut unique = true;
        let mut kept = 0;
        let mut at = 0;
        while at < by_status.len() {
            let end = by_status[at..]
                .iter()
                .position(|&index| status(index) != status(by_status[at]))
                .map_or(by_status.len(), |offset| at + offset);
            if end - at == 1 {
                by_status[kept] = by_status[at];
                kept += 1;
            } else {
                unique = false;
            }
            at = end;
        }
        by_status.truncate(kept);
        unique
    }

    /// Rows of the state table: those before the first province's.
    fn state_rows(&self) -> usize {
        self.entries
            .iter()
            .find(|entry| entry.certificate.location.province)
            .map_or(self.rows.len(), |entry| entry.rows.start)
    }

    fn measure(&self) -> usize {
        self.entries.capacity() * size_of::<Entry>()
            + self.by_status.capacity() * size_of::<usize>()
            + self.rows.capacity() * size_of::<cbuilding::Row>()
    }
}

/// One planned status: its certificate, carryable held rows and output
/// position across both tables.
#[derive(Clone, Debug, Default)]
struct Plan {
    certificate: Option<Certificate>,
    held: Option<Range<usize>>,
    start: usize,
}

/// The token of a status whose plan could not be stored.
const UNPLANNED: usize = usize::MAX;

/// What lets an hour carry both tables without walking the locations: the
/// watch epoch taken before the latest walk that held every status under a
/// certificate, and the two location table headers read at that walk.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Settled {
    epoch: u64,
    roots: [[u8; 16]; 2],
}

/// Rows of the latest complete walk and the walk being made.
#[derive(Default)]
pub struct BuildingStore {
    current: Held,
    previous: Held,
    /// This walk's plans, indexed by token.
    plans: Vec<Plan>,
    counts: Counts,
    /// Locations this walk planned, without buildings too, for sizing.
    walked: usize,
    /// Rows this walk planned in both tables: the next status's position.
    planned: usize,
    /// Rows this walk planned in the state table.
    state_rows: usize,
    /// Every status this walk planned holds a certificate and its rows.
    held_all: bool,
    settled: Option<Settled>,
    /// The statuses this hour's take visited.
    touched: Vec<usize>,
    /// A settled hour's held entries the take visited, in walk order.
    marked: Vec<usize>,
    /// Their held locations, sequences and fresh locations.
    held: Vec<Location>,
    sequences: Vec<u64>,
    fresh: Vec<Location>,
    /// A settled hour's statuses read afresh.
    scratch: Vec<cbuilding::Row>,
    /// Allocated bytes as of the last finish, for the game thread.
    bytes: usize,
    /// Set on the game thread; the next walk drops every held row.
    forgotten: bool,
}

impl BuildingStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Nothing held certifies anything: the next capture walks and reads
    /// afresh. Allocation-free for the game thread.
    pub fn forget(&mut self) {
        self.forgotten = true;
        self.settled = None;
    }

    /// As of the last finish, so the game thread need not measure.
    pub fn allocated_bytes(&self) -> usize {
        self.bytes
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn counts(&self) -> Counts {
        self.counts
    }

    fn begin(&mut self) {
        if std::mem::take(&mut self.forgotten) {
            self.current.clear();
            self.previous.clear();
        }
        std::mem::swap(&mut self.current, &mut self.previous);
        self.current.clear();
        self.plans.clear();
        self.counts = Counts::default();
        self.walked = 0;
        self.planned = 0;
        self.state_rows = 0;
        self.held_all = true;
        self.settled = None;
    }

    fn measure(&mut self) {
        self.bytes = self.current.measure()
            + self.previous.measure()
            + self.plans.capacity() * size_of::<Plan>()
            + (self.touched.capacity() + self.marked.capacity()) * size_of::<usize>()
            + (self.held.capacity() + self.fresh.capacity()) * size_of::<Location>()
            + self.sequences.capacity() * size_of::<u64>()
            + self.scratch.capacity() * size_of::<cbuilding::Row>();
    }

    /// Keeps this walk's rows only when both tables were read or carried
    /// completely, and publishes its counts. `candidate` is the watch epoch
    /// and location table headers taken before the walk; they let the next
    /// hours carry both tables without walking when the walk held every
    /// status once.
    fn finish(&mut self, walk: Walk, candidate: Option<(u64, [[u8; 16]; 2])>) {
        let complete = matches!(walk.states, Outcome::Rows { .. })
            && matches!(walk.provinces, Outcome::Rows { .. });
        let unique = if complete {
            self.current.order()
        } else {
            self.current.clear();
            false
        };
        self.previous.clear();
        self.plans.clear();
        let states = walk.states.len();
        self.settled = candidate
            .filter(|_| {
                complete
                    && self.held_all
                    && unique
                    && self.state_rows == states
                    && self.current.rows.len() == states + walk.provinces.len()
            })
            .map(|(epoch, roots)| Settled { epoch, roots });
        self.measure();
        crate::building_changes::record(&self.counts, complete.then_some(self.walked));
    }

    /// Every status a hooked writer touched since the previous take, kept
    /// for this hour, and the epoch the take certifies; `None` when it
    /// certifies nothing or a status could not be kept.
    fn take(&mut self, changes: &dyn Changes) -> Option<u64> {
        let touched = &mut self.touched;
        touched.clear();
        let mut kept = true;
        let epoch = changes.take(&mut |status| {
            kept &= allocation::advisory(|| allocation::reserve(touched, 1)).is_ok();
            if kept {
                touched.push(status);
            }
        });
        epoch.filter(|_| kept)
    }

    /// The settled hour this capture may take: the take certifies the epoch
    /// of the walk that settled the store, and both location tables read as
    /// then.
    fn settled_hour(&self, taken: Option<u64>, roots: Option<[[u8; 16]; 2]>) -> Option<Settled> {
        let settled = self.settled?;
        (!self.forgotten
            && taken == Some(settled.epoch)
            && roots == Some(settled.roots))
        .then_some(settled)
    }

    /// The held statuses the take visited, read afresh after registering
    /// each in `epoch`, with their held locations, sequences and fresh
    /// locations; `false` when one cannot be registered in that epoch or
    /// does not read, or storage cannot grow.
    fn refresh(
        &mut self,
        memory: &impl Memory,
        base: usize,
        changes: &dyn Changes,
        epoch: u64,
    ) -> bool {
        let Self {
            current,
            marked,
            held,
            sequences,
            fresh,
            scratch,
            ..
        } = self;
        held.clear();
        sequences.clear();
        fresh.clear();
        let reserved = allocation::advisory(|| {
            allocation::reserve(held, marked.len())?;
            allocation::reserve(sequences, marked.len())?;
            allocation::reserve(fresh, marked.len())
        });
        if reserved.is_err() {
            return false;
        }
        for &index in marked.iter() {
            let location = current.entries[index].certificate.location;
            // Registration precedes the read it certifies.
            match changes.sequence(location.status) {
                Some((at, sequence)) if at == epoch => sequences.push(sequence),
                _ => return false,
            }
            held.push(location);
            fresh.push(location);
        }
        cbuilding_status::refresh(memory, base, held, fresh, scratch).is_ok()
    }

    /// Holds the refreshed statuses' rows at their place in walk order,
    /// their certificates taken in `epoch`; `false` when storage cannot
    /// grow. Rows are rewritten in place unless a building count changed.
    fn hold_refreshed(&mut self, epoch: u64) -> bool {
        let Self {
            current,
            previous,
            marked,
            sequences,
            fresh,
            scratch,
            ..
        } = self;
        let refreshed = marked.iter().zip(sequences.iter().zip(fresh.iter()));
        let resized = marked
            .iter()
            .zip(fresh.iter())
            .any(|(&index, fresh)| current.entries[index].rows.len() != fresh.buildings);
        let mut at = 0;
        if !resized {
            for (&index, (&sequence, &location)) in refreshed {
                let entry = &mut current.entries[index];
                let end = at + location.buildings;
                current.rows[entry.rows.clone()].copy_from_slice(&scratch[at..end]);
                entry.certificate = Certificate {
                    epoch,
                    sequence,
                    location,
                };
                at = end;
            }
            return true;
        }
        let held_rows: usize = marked
            .iter()
            .map(|&index| current.entries[index].rows.len())
            .sum();
        let fresh_rows: usize = fresh.iter().map(|location| location.buildings).sum();
        let rows = current.rows.len() - held_rows + fresh_rows;
        previous.clear();
        let reserved = allocation::advisory(|| {
            allocation::reserve(&mut previous.entries, current.entries.len())?;
            allocation::reserve(&mut previous.rows, rows)
        });
        if reserved.is_err() {
            return false;
        }
        let mut refreshed = refreshed.peekable();
        for (index, entry) in current.entries.iter().enumerate() {
            let start = previous.rows.len();
            let certificate = match refreshed.next_if(|&(&next, _)| next == index) {
                Some((_, (&sequence, &location))) => {
                    let end = at + location.buildings;
                    previous.rows.extend_from_slice(&scratch[at..end]);
                    at = end;
                    Certificate {
                        epoch,
                        sequence,
                        location,
                    }
                }
                None => {
                    previous
                        .rows
                        .extend_from_slice(&current.rows[entry.rows.clone()]);
                    entry.certificate
                }
            };
            previous.entries.push(Entry {
                certificate,
                rows: start..previous.rows.len(),
                order: entry.order,
            });
        }
        // The same statuses in the same order: the index carries over.
        std::mem::swap(&mut previous.by_status, &mut current.by_status);
        std::mem::swap(current, previous);
        previous.clear();
        true
    }

    /// A settled hour: the touched statuses read afresh and both tables
    /// published from the held rows. `None` walks.
    fn settle(
        &mut self,
        memory: &impl Memory,
        base: usize,
        changes: &dyn Changes,
        settled: Settled,
        states: &mut Vec<cbuilding::Row>,
        provinces: &mut Vec<cbuilding::Row>,
    ) -> Option<Walk> {
        // Statuses the take visited that no entry holds are not walked.
        let Self {
            current,
            touched,
            marked,
            ..
        } = self;
        marked.clear();
        allocation::advisory(|| allocation::reserve(marked, touched.len())).ok()?;
        marked.extend(touched.iter().filter_map(|&status| current.index(status)));
        marked.sort_unstable();
        if !self.refresh(memory, base, changes, settled.epoch)
            || !self.hold_refreshed(settled.epoch)
        {
            return None;
        }
        let held = &self.current;
        let (state_rows, province_rows) = held.rows.split_at(held.state_rows());
        if !(publish(states, state_rows) && publish(provinces, province_rows)) {
            return None;
        }
        let walk = Walk {
            states: Outcome::Rows {
                len: state_rows.len(),
            },
            provinces: Outcome::Rows {
                len: province_rows.len(),
            },
        };
        let refreshed_rows: usize = self.fresh.iter().map(|location| location.buildings).sum();
        let with_rows = held
            .entries
            .iter()
            .filter(|entry| !entry.rows.is_empty())
            .count() as u64;
        let refreshed_with_rows = self
            .fresh
            .iter()
            .filter(|location| location.buildings != 0)
            .count() as u64;
        let read_rows = refreshed_rows as u64;
        self.counts = Counts {
            reused: with_rows - refreshed_with_rows,
            read: self.marked.len() as u64,
            reused_rows: held.rows.len() as u64 - read_rows,
            read_rows,
            empty: held.entries.len() as u64 - with_rows,
            marked: self.marked.len() as u64,
            settled: true,
            quiet: self.marked.is_empty(),
            ..Counts::default()
        };
        self.measure();
        crate::building_changes::record(&self.counts, None);
        Some(walk)
    }
}

/// `rows` into the front of `output`, growing it as the walk grows it.
fn publish(output: &mut Vec<cbuilding::Row>, rows: &[cbuilding::Row]) -> bool {
    if rows.len() > output.len() {
        if allocation::reserve(output, rows.len() - output.len()).is_err() {
            return false;
        }
        output.resize(rows.len(), cbuilding::Row::default());
    }
    output[..rows.len()].copy_from_slice(rows);
    true
}

/// Both tables for one hour, carrying through `store`: a settled hour reads
/// only the statuses a hooked writer touched; any other hour walks every
/// location and carries each status it proves current.
pub fn capture(
    memory: &impl Memory,
    base: usize,
    game_state: usize,
    states: &mut Vec<cbuilding::Row>,
    provinces: &mut Vec<cbuilding::Row>,
    store: &mut BuildingStore,
    changes: &dyn Changes,
) -> Walk {
    // Taken before any registration or read this capture certifies; a
    // status a writer touches after the take is visited by the next one.
    let roots = cbuilding_status::roots(memory, game_state);
    let taken = store.take(changes);
    if let Some(settled) = store.settled_hour(taken, roots)
        && let Some(walk) = store.settle(memory, base, changes, settled, states, provinces)
    {
        return walk;
    }
    let mut reuse = BuildingReuse::begin(store, changes);
    let walk =
        cbuilding_status::walk_carried(memory, base, game_state, states, provinces, &mut reuse);
    reuse.store.finish(walk, taken.zip(roots));
    walk
}

/// One walk's view of the store and the observation that certifies reuse.
struct BuildingReuse<'a> {
    store: &'a mut BuildingStore,
    changes: &'a dyn Changes,
}

impl<'a> BuildingReuse<'a> {
    fn begin(store: &'a mut BuildingStore, changes: &'a dyn Changes) -> Self {
        store.begin();
        Self { store, changes }
    }
}

impl Carry for BuildingReuse<'_> {
    /// Registration precedes every read it certifies. A status without
    /// buildings is registered and held too, so that a settled hour's take
    /// visits it once a writer touches it, but staged rather than carried, so
    /// the walk checks its active vector as it checks every one.
    fn plan(&mut self, location: &Location) -> (usize, bool) {
        let store = &mut *self.store;
        store.walked += 1;
        if location.buildings == 0 {
            store.counts.empty += 1;
        }
        let start = store.planned;
        store.planned = start.saturating_add(location.buildings);
        if !location.province {
            store.state_rows = store.state_rows.saturating_add(location.buildings);
        }
        let certificate = self
            .changes
            .sequence(location.status)
            .map(|(epoch, sequence)| Certificate {
                epoch,
                sequence,
                location: *location,
            });
        let held = certificate
            .and_then(|certificate| store.previous.find(&certificate))
            .filter(|held| held.len() == location.buildings);
        let carry = held.is_some() && location.buildings != 0;
        let token = store.plans.len();
        let planned = allocation::advisory(|| allocation::reserve(&mut store.plans, 1));
        if planned.is_err() || certificate.is_none() {
            store.held_all = false;
        }
        if planned.is_err() {
            // Unplanned: read afresh and held for nothing.
            return (UNPLANNED, false);
        }
        store.plans.push(Plan {
            certificate,
            held,
            start,
        });
        (token, carry)
    }

    fn carry(&mut self, token: usize, output: &mut [cbuilding::Row]) {
        let BuildingStore {
            previous,
            current,
            plans,
            counts,
            held_all,
            ..
        } = &mut *self.store;
        let Some(Plan {
            certificate: Some(certificate),
            held: Some(range),
            start,
        }) = plans.get(token).cloned()
        else {
            return;
        };
        let rows = &previous.rows[range];
        output.copy_from_slice(rows);
        counts.reused += 1;
        counts.reused_rows += rows.len() as u64;
        // A failed hold only withholds the next hour's certificate.
        *held_all &= current.hold(certificate, token, rows, start);
    }

    fn read(&mut self, token: usize, rows: &[cbuilding::Row]) {
        let store = &mut *self.store;
        if !rows.is_empty() {
            store.counts.read += 1;
            store.counts.read_rows += rows.len() as u64;
        }
        let Some(plan) = store.plans.get(token).cloned() else {
            return;
        };
        if let Some(certificate) = plan.certificate {
            store.held_all &= store.current.hold(certificate, token, rows, plan.start);
        }
    }
}
