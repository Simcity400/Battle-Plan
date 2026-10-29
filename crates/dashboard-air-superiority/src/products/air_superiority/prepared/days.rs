//! A timeline view's classified days, kept across its queries. Classifying a day changes the
//! side catalogue only at that day's hour, so each kept day holds its sample and those changes:
//! a later query takes the days it shares, replays their changes onto the catalogue it starts
//! from, and classifies only the days after them, exactly as classifying every day in turn does.
//! A source keeps its views' days within one budget, every branch and view together.
use super::*;
use std::sync::{
    Mutex, PoisonError,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

/// Estimated bytes of the days a source keeps for its timelines, every branch and view together.
const KEPT_BYTES: usize = 128 << 20;
/// A view's estimated bytes before any of its days.
const VIEW_BYTES: usize = 1 << 10;

/// A timeline view: its branch, the prepared state it reads ([`serial`]), its first hour, its
/// viewpoint and whether that is a country.
pub(super) type View = (u64, u64, i32, Option<String>, bool);

#[cfg(test)]
thread_local! {
    /// Days classified on this thread.
    pub(super) static CLASSIFIED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A serial naming one read of a branch's prepared state, so its days are kept apart from those
/// of the state before it.
pub(super) fn serial() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// `source`'s timelines, which all its queries share.
pub(super) fn timelines(source: &Source) -> Arc<Timelines> {
    #[derive(Default)]
    struct Shared(Arc<Timelines>);
    let shared = source.state::<Shared>();
    let shared = shared.lock().unwrap_or_else(PoisonError::into_inner);
    Arc::clone(&shared.0)
}

/// The days kept for each view, the most recently used first, and the bytes they are counted
/// as, all within one budget.
pub(super) struct Timelines {
    views: Mutex<Vec<(View, Arc<Kept>)>>,
    bytes: Arc<AtomicUsize>,
    budget: usize,
    /// The views whose later days a thread is classifying ahead of their queries.
    ahead: Mutex<std::collections::HashSet<View>>,
}

impl Default for Timelines {
    fn default() -> Self {
        Self::with_budget(KEPT_BYTES)
    }
}

/// A view's days claimed for classifying ahead of its queries, released when dropped.
pub(super) struct Claim {
    timelines: Arc<Timelines>,
    view: View,
}

impl Drop for Claim {
    fn drop(&mut self) {
        let mut ahead = self
            .timelines
            .ahead
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        ahead.remove(&self.view);
    }
}

/// One view's kept days, and the bytes they and the view are counted as, returned to the
/// budget when the view is let go.
pub(super) struct Kept {
    pub(super) days: Mutex<Days>,
    bytes: AtomicUsize,
    total: Arc<AtomicUsize>,
    /// Whether the view is kept: one the budget had no room for keeps no day.
    listed: bool,
    /// Queries waiting for these days or reading them: work done ahead gives way to them.
    queries: AtomicUsize,
}

/// A query counted among those waiting for or reading a view's days while it is held.
pub(super) struct Waiting<'a>(&'a Kept);

impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        self.0.queries.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Drop for Kept {
    fn drop(&mut self) {
        self.total
            .fetch_sub(*self.bytes.get_mut(), Ordering::AcqRel);
    }
}

impl Kept {
    fn new(total: &Arc<AtomicUsize>, listed: bool) -> Self {
        Self {
            days: Mutex::default(),
            bytes: AtomicUsize::new(0),
            total: Arc::clone(total),
            listed,
            queries: AtomicUsize::new(0),
        }
    }

    /// Counts a query among those waiting for or reading these days while the result is held.
    pub(super) fn waiting(&self) -> Waiting<'_> {
        self.queries.fetch_add(1, Ordering::AcqRel);
        Waiting(self)
    }

    /// Whether a query waits for these days or reads them.
    pub(super) fn wanted(&self) -> bool {
        self.queries.load(Ordering::Acquire) > 0
    }

    fn release(&self, bytes: usize) {
        self.bytes.fetch_sub(bytes, Ordering::AcqRel);
        self.total.fetch_sub(bytes, Ordering::AcqRel);
    }
}

impl Timelines {
    fn with_budget(budget: usize) -> Self {
        Self {
            views: Mutex::default(),
            bytes: Arc::default(),
            budget,
            ahead: Mutex::default(),
        }
    }

    /// Claims `view`'s days for classifying ahead of its queries, until the claim is dropped:
    /// `None` while another claim holds them.
    pub(super) fn claim_ahead(self: &Arc<Self>, view: &View) -> Option<Claim> {
        let mut ahead = self.ahead.lock().unwrap_or_else(PoisonError::into_inner);
        ahead.insert(view.clone()).then(|| Claim {
            timelines: Arc::clone(self),
            view: view.clone(),
        })
    }

    /// `view`'s kept days, if it is kept: it admits no view.
    pub(super) fn listed(&self, view: &View) -> Option<Arc<Kept>> {
        let views = self.views.lock().unwrap_or_else(PoisonError::into_inner);
        let (_, kept) = views.iter().find(|(held, _)| held == view)?;
        Some(Arc::clone(kept))
    }

    /// `view`'s kept days, from now on the most recently used. Views of the branch's earlier
    /// prepared states that no query holds are let go. A new view is kept only once the budget
    /// has room for it, letting go of the least recently used views no query holds; else it is
    /// answered as one that keeps nothing.
    pub(super) fn view(&self, view: View) -> Arc<Kept> {
        let stale = {
            let mut views = self.views.lock().unwrap_or_else(PoisonError::into_inner);
            let (stale, live): (Vec<_>, Vec<_>) = std::mem::take(&mut *views)
                .into_iter()
                .partition(|(held, kept)| {
                    held.0 == view.0 && held.1 != view.1 && Arc::strong_count(kept) == 1
                });
            *views = live;
            if let Some(kept) = Self::used(&mut views, &view) {
                return kept;
            }
            stale
        };
        drop(stale);
        let kept = Arc::new(Kept::new(&self.bytes, true));
        if !self.reserve(&kept, VIEW_BYTES + view.3.as_ref().map_or(0, String::len)) {
            return Arc::new(Kept::new(&self.bytes, false));
        }
        let mut views = self.views.lock().unwrap_or_else(PoisonError::into_inner);
        // Another query may have kept the view meanwhile; this one's charge goes with it.
        if let Some(listed) = Self::used(&mut views, &view) {
            return listed;
        }
        views.insert(0, (view, Arc::clone(&kept)));
        kept
    }

    /// `view`'s kept days in `views`, moved to the front, if they are kept.
    fn used(views: &mut Vec<(View, Arc<Kept>)>, view: &View) -> Option<Arc<Kept>> {
        let at = views.iter().position(|(held, _)| held == view)?;
        let entry = views.remove(at);
        let kept = Arc::clone(&entry.1);
        views.insert(0, entry);
        Some(kept)
    }

    /// Counts `bytes` more for `kept`, letting go of the least recently used views no query
    /// holds, other than `kept`, while they would pass the budget. False, counting nothing,
    /// when they still would.
    fn reserve(&self, kept: &Arc<Kept>, bytes: usize) -> bool {
        loop {
            let held = self.bytes.load(Ordering::Acquire);
            if held.saturating_add(bytes) <= self.budget {
                let counted = self.bytes.compare_exchange(
                    held,
                    held + bytes,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                );
                if counted.is_ok() {
                    kept.bytes.fetch_add(bytes, Ordering::AcqRel);
                    return true;
                }
            } else if !self.evict(kept) {
                return false;
            }
        }
    }

    /// Lets go of the least recently used view no query holds, other than `kept`: false when
    /// there is none.
    fn evict(&self, kept: &Arc<Kept>) -> bool {
        let evicted = {
            let mut views = self.views.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(at) = views
                .iter()
                .rposition(|(_, held)| !Arc::ptr_eq(held, kept) && Arc::strong_count(held) == 1)
            else {
                return false;
            };
            views.remove(at).1
        };
        drop(evicted);
        true
    }
}

/// One view's classified days from its first, in order.
#[derive(Default)]
pub(super) struct Days {
    days: Vec<Day>,
    /// The bytes the days are counted as.
    bytes: usize,
    /// A day could not be kept within the budget: this query keeps no later one.
    full: bool,
}

struct Day {
    sample: Arc<Snapshot>,
    changes: Arc<[Change]>,
}

/// A change classifying a day made to the side catalogue.
enum Change {
    /// A side first met that day, as the day left it.
    Side(AirSide),
    /// A side's read at that day's hour.
    Read(u32, model::AirSideRead),
}

impl Change {
    fn apply(&self, sides: &mut Vec<AirSide>) {
        match self {
            Change::Side(side) => sides.push(side.clone()),
            Change::Read(index, read) => {
                let reads = &mut sides[*index as usize].reads;
                let at = reads.partition_point(|held| held.0 <= read.0);
                match at
                    .checked_sub(1)
                    .filter(|&before| reads[before].0 == read.0)
                {
                    Some(before) => reads[before] = read.clone(),
                    None => reads.insert(at, read.clone()),
                }
            }
        }
    }
}

/// The side catalogue as classifying a day found it: each side's count of reads, and the
/// reads it already held at the day's hour.
pub(super) struct Before {
    reads: Vec<usize>,
    at_hour: Vec<(usize, model::AirSideRead)>,
}

fn read_at(side: &AirSide, hour: i32) -> Option<&model::AirSideRead> {
    let at = side.reads.partition_point(|read| read.0 <= hour);
    at.checked_sub(1)
        .map(|at| &side.reads[at])
        .filter(|read| read.0 == hour)
}

impl Before {
    fn new(sides: &[AirSide], hour: i32) -> Self {
        Self {
            reads: sides.iter().map(|side| side.reads.len()).collect(),
            at_hour: sides
                .iter()
                .enumerate()
                .filter_map(|(index, side)| read_at(side, hour).map(|read| (index, read.clone())))
                .collect(),
        }
    }

    /// What classifying the day at `hour` changed, now that `sides` holds its classification.
    fn changes(self, sides: &[AirSide], hour: i32) -> Vec<Change> {
        let mut changes = Vec::new();
        for (index, side) in sides.iter().enumerate() {
            let Some(&reads) = self.reads.get(index) else {
                changes.push(Change::Side(side.clone()));
                continue;
            };
            let Some(read) = read_at(side, hour) else {
                continue;
            };
            let held = self
                .at_hour
                .iter()
                .find_map(|(at, held)| (*at == index).then_some(held));
            if side.reads.len() != reads || held != Some(read) {
                changes.push(Change::Read(index as u32, read.clone()));
            }
        }
        changes
    }
}

impl Days {
    /// How many days are kept.
    pub(super) fn len(&self) -> usize {
        self.days.len()
    }

    /// The kept samples of the first of `complete`, the days a query shares with every later
    /// one, with their changes applied to `sides`. Days kept for other hours, as a store
    /// answering otherwise would give, are let go from `kept`.
    pub(super) fn resume(
        &mut self,
        kept: &Kept,
        complete: &[i32],
        sides: &mut Vec<AirSide>,
        cancelled: &AtomicBool,
    ) -> io::Result<Vec<Arc<Snapshot>>> {
        self.full = false;
        let shared = self.days.len().min(complete.len());
        let days = &self.days[..shared];
        if days
            .iter()
            .zip(complete)
            .any(|(day, &hour)| day.sample.hour != hour)
        {
            kept.release(self.bytes);
            *self = Self::default();
            return Ok(Vec::new());
        }
        for day in days {
            check_cancel(cancelled)?;
            for change in day.changes.iter() {
                change.apply(sides);
            }
        }
        Ok(days.iter().map(|day| Arc::clone(&day.sample)).collect())
    }

    /// The catalogue before classifying the day at `at` of the query's days, at `hour`, when
    /// that day is to be kept: `kept` is a kept view, the day is among the `complete` ones and
    /// follows the kept days, and the query has not found the budget full.
    pub(super) fn before(
        &self,
        kept: &Kept,
        at: usize,
        complete: usize,
        sides: &[AirSide],
        hour: i32,
    ) -> Option<Before> {
        (kept.listed && !self.full && at < complete && at == self.days.len())
            .then(|| Before::new(sides, hour))
    }

    /// Keeps the day `sample` classified, which left `sides` as they are, when `timelines` has
    /// room for it in `kept`.
    pub(super) fn keep(
        &mut self,
        timelines: &Timelines,
        kept: &Arc<Kept>,
        sample: &Arc<Snapshot>,
        before: Before,
        sides: &[AirSide],
    ) {
        let changes = before.changes(sides, sample.hour);
        let bytes = day_bytes(sample, self.days.last().map(|day| &*day.sample), &changes);
        if !timelines.reserve(kept, bytes) {
            self.full = true;
            return;
        }
        self.bytes += bytes;
        self.days.push(Day {
            sample: Arc::clone(sample),
            changes: changes.into(),
        });
    }
}

/// Work classifying a view's days ahead of its queries: the side catalogue as the kept days it
/// applied left it, so it carries on from them rather than replaying them. It holds the view's
/// lock only to take the days queries kept since and to add each day it classifies.
#[derive(Default)]
pub(super) struct Ahead {
    sides: Vec<AirSide>,
    /// How many of the kept days `sides` holds the changes of: none before the first are applied
    /// and once a day it classified was not kept, so it starts over from the base catalogue.
    applied: Option<usize>,
    /// The last of them.
    last: Option<Arc<Snapshot>>,
}

/// Kept days work ahead has not applied, taken under the view's lock to apply after it.
pub(super) struct Unapplied {
    /// How many kept days they follow: none when they are all of them.
    from: Option<usize>,
    days: Vec<(Arc<Snapshot>, Arc<[Change]>)>,
}

/// What keeping a day classified ahead of a view's queries did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Keep {
    Kept,
    /// The kept days are no longer those the work applied: a query kept or let go of days.
    Moved,
    /// The budget has no room for the day.
    Full,
}

impl Ahead {
    /// Whether the first `applied` of `days` are those this applied.
    fn follows(&self, days: &Days, applied: usize) -> bool {
        match applied.checked_sub(1) {
            None => true,
            Some(at) => days
                .days
                .get(at)
                .zip(self.last.as_ref())
                .is_some_and(|(day, last)| Arc::ptr_eq(&day.sample, last)),
        }
    }

    /// The kept `days` this has not applied: all of them once those it applied are not the
    /// first kept.
    pub(super) fn unapplied(&self, days: &Days) -> Unapplied {
        let from = self.applied.filter(|&applied| self.follows(days, applied));
        Unapplied {
            from,
            days: days.days[from.unwrap_or(0)..]
                .iter()
                .map(|day| (Arc::clone(&day.sample), Arc::clone(&day.changes)))
                .collect(),
        }
    }

    /// Applies `unapplied` to the catalogue, from `base` when they are all the kept days: false,
    /// applying nothing, when they are not the days of `wanted` that follow those applied.
    pub(super) fn apply(&mut self, unapplied: Unapplied, base: &[AirSide], wanted: &[i32]) -> bool {
        let from = unapplied.from.unwrap_or(0);
        let follow = wanted.get(from..).is_some_and(|hours| {
            hours.len() >= unapplied.days.len()
                && unapplied
                    .days
                    .iter()
                    .zip(hours)
                    .all(|((sample, _), &hour)| sample.hour == hour)
        });
        if !follow {
            return false;
        }
        if unapplied.from.is_none() {
            self.sides = base.to_vec();
            self.last = None;
        }
        let applied = from + unapplied.days.len();
        for (sample, changes) in unapplied.days {
            for change in changes.iter() {
                change.apply(&mut self.sides);
            }
            self.last = Some(sample);
        }
        self.applied = Some(applied);
        true
    }

    /// How many kept days the catalogue holds the changes of.
    pub(super) fn applied(&self) -> usize {
        self.applied.unwrap_or(0)
    }

    /// The catalogue, which classifying the next day extends.
    pub(super) fn sides(&mut self) -> &mut Vec<AirSide> {
        &mut self.sides
    }

    /// The catalogue before classifying the next day, at `hour`.
    pub(super) fn before(&self, hour: i32) -> Before {
        Before::new(&self.sides, hour)
    }

    /// Keeps `sample`, the next day, classified from the catalogue as `before` found it, after
    /// `kept`'s days when `timelines` has room for it and they are still those this applied.
    /// Its room is made, letting go of other views, before the view's lock is taken, which then
    /// only checks and adds the day. Else the catalogue, which holds the day, starts over from
    /// the base.
    pub(super) fn keep(
        &mut self,
        timelines: &Timelines,
        kept: &Arc<Kept>,
        sample: Arc<Snapshot>,
        before: Before,
    ) -> Keep {
        let changes = before.changes(&self.sides, sample.hour);
        let bytes = day_bytes(&sample, self.last.as_deref(), &changes);
        let day = Day {
            sample: Arc::clone(&sample),
            changes: changes.into(),
        };
        let Some(applied) = self.applied.take() else {
            return Keep::Moved;
        };
        if !timelines.reserve(kept, bytes) {
            return Keep::Full;
        }
        let mut days = kept.days.lock().unwrap_or_else(PoisonError::into_inner);
        if applied != days.days.len() || !self.follows(&days, applied) {
            drop(days);
            kept.release(bytes);
            return Keep::Moved;
        }
        days.bytes += bytes;
        days.days.push(day);
        drop(days);
        self.applied = Some(applied + 1);
        self.last = Some(sample);
        Keep::Kept
    }
}

/// A kept day's estimated bytes: its sample with what the sample owns, the reads it does not
/// share with the day before, and its changes.
fn day_bytes(sample: &Snapshot, previous: Option<&Snapshot>, changes: &[Change]) -> usize {
    use dashboard_base::prepared::query::observations::serde_bytes;
    use std::mem::size_of;
    let theatres = match previous {
        Some(previous) if Arc::ptr_eq(&previous.theatres, &sample.theatres) => 0,
        _ => serde_bytes(&*sample.theatres),
    };
    let key_read = match (
        &sample.key_read,
        previous.and_then(|day| day.key_read.as_ref()),
    ) {
        (Some(read), Some(before)) if Arc::ptr_eq(read, before) => 0,
        (Some(read), _) => size_of::<scan::KeyRead>() + serde_bytes(&**read),
        (None, _) => 0,
    };
    let changes: usize = changes
        .iter()
        .map(|change| {
            size_of::<Change>()
                + match change {
                    Change::Side(side) => serde_bytes(side),
                    Change::Read(_, read) => serde_bytes(read),
                }
        })
        .sum();
    size_of::<Day>()
        + size_of::<Snapshot>()
        + sample.cells.len() * size_of::<(u32, model::AirCell)>()
        + serde_bytes(&sample.cells)
        + serde_bytes(&sample.fallback)
        + sample.wings.len() * size_of::<(u32, u32)>()
        + serde_bytes(&sample.wings)
        + sample.active.len() * size_of::<u32>()
        + serde_bytes(&sample.active)
        + theatres
        + key_read
        + changes
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(branch: u64, serial: u64, first: i32) -> View {
        (branch, serial, first, None, false)
    }

    #[test]
    fn kept_views_stay_within_their_budget_letting_go_of_the_oldest_no_query_holds() {
        let timelines = Timelines::with_budget(10 * VIEW_BYTES);
        let first = timelines.view(view(1, 0, 0));
        assert!(timelines.reserve(&first, 4 * VIEW_BYTES));
        let second = timelines.view(view(1, 0, 24));
        assert!(timelines.reserve(&second, 4 * VIEW_BYTES));
        drop(first);
        let third = timelines.view(view(1, 0, 48));
        // The first view, the oldest no query holds, makes room for the third's days.
        assert!(timelines.reserve(&third, 4 * VIEW_BYTES));
        let views = timelines.views.lock().unwrap();
        let firsts: Vec<i32> = views.iter().map(|(held, _)| held.2).collect();
        assert_eq!(firsts, [48, 24]);
        drop(views);
        assert_eq!(timelines.bytes.load(Ordering::Acquire), 10 * VIEW_BYTES);
        // A view a query holds is never let go, so what does not fit is not kept.
        assert!(!timelines.reserve(&third, 2 * VIEW_BYTES));
        assert_eq!(timelines.bytes.load(Ordering::Acquire), 10 * VIEW_BYTES);
        // With every view held, a new one is not kept and charges nothing.
        let fourth = timelines.view(view(1, 0, 72));
        assert!(!fourth.listed);
        assert_eq!(timelines.bytes.load(Ordering::Acquire), 10 * VIEW_BYTES);
        assert_eq!(timelines.views.lock().unwrap().len(), 2);
        drop((second, third, fourth));
        assert_eq!(timelines.bytes.load(Ordering::Acquire), 10 * VIEW_BYTES);
        drop(
            timelines
                .views
                .lock()
                .unwrap()
                .drain(..)
                .collect::<Vec<_>>(),
        );
        assert_eq!(timelines.bytes.load(Ordering::Acquire), 0);
    }

    fn sample(hour: i32) -> Arc<Snapshot> {
        Arc::new(Snapshot {
            hour,
            key_read: None,
            cells: BTreeMap::new(),
            fallback: model::AirCell::default(),
            wings: BTreeMap::new(),
            wings_known: true,
            complete: true,
            theatres: Arc::default(),
            theatres_known: true,
            available: true,
            excluded: 0,
            active: BTreeSet::new(),
            theatre_read: None,
        })
    }

    #[test]
    fn work_ahead_keeps_a_day_only_after_the_days_it_applied() {
        let timelines = Timelines::default();
        let kept = timelines.view(view(1, 0, 0));
        let wanted = [23, 47, 71];
        let take = |ahead: &mut Ahead, wanted: &[i32]| {
            let unapplied = ahead.unapplied(&kept.days.lock().unwrap());
            ahead.apply(unapplied, &[], wanted)
        };
        let (mut first, mut second) = (Ahead::default(), Ahead::default());
        assert!(take(&mut first, &wanted) && take(&mut second, &wanted));
        let before = first.before(23);
        assert_eq!(
            first.keep(&timelines, &kept, sample(23), before),
            Keep::Kept
        );
        // The days moved on since the second applied them: it keeps nothing, counts nothing
        // and starts over.
        let counted = timelines.bytes.load(Ordering::Acquire);
        let before = second.before(23);
        assert_eq!(
            second.keep(&timelines, &kept, sample(23), before),
            Keep::Moved
        );
        assert_eq!(kept.days.lock().unwrap().len(), 1);
        assert_eq!(timelines.bytes.load(Ordering::Acquire), counted);
        assert!(take(&mut second, &wanted));
        assert_eq!(second.applied(), 1);
        let before = second.before(47);
        assert_eq!(
            second.keep(&timelines, &kept, sample(47), before),
            Keep::Kept
        );
        // The first takes only the day kept since, after the one it applied.
        let unapplied = first.unapplied(&kept.days.lock().unwrap());
        assert_eq!((unapplied.from, unapplied.days.len()), (Some(1), 1));
        assert!(first.apply(unapplied, &[], &wanted));
        assert_eq!(first.applied(), 2);
        // Days kept for other hours are not the ones work ahead classifies: it stops.
        assert!(!take(&mut Ahead::default(), &[23, 48, 71]));
        assert!(!take(&mut Ahead::default(), &[23]));
        // A day the budget has no room for is not kept.
        let full = Timelines::with_budget(VIEW_BYTES);
        let kept = full.view(view(1, 0, 0));
        let mut ahead = Ahead::default();
        let unapplied = ahead.unapplied(&kept.days.lock().unwrap());
        assert!(ahead.apply(unapplied, &[], &wanted));
        let before = ahead.before(23);
        assert_eq!(ahead.keep(&full, &kept, sample(23), before), Keep::Full);
        assert_eq!(kept.days.lock().unwrap().len(), 0);
        assert_eq!(full.bytes.load(Ordering::Acquire), VIEW_BYTES);
    }

    #[test]
    fn a_branchs_new_prepared_state_lets_go_of_the_views_of_its_earlier_one() {
        let timelines = Timelines::default();
        let earlier = timelines.view(view(1, 0, 0));
        assert!(timelines.reserve(&earlier, 4 * VIEW_BYTES));
        let other = timelines.view(view(2, 0, 0));
        drop(earlier);
        let current = timelines.view(view(1, 1, 0));
        let views = timelines.views.lock().unwrap();
        let held: Vec<(u64, u64)> = views.iter().map(|(held, _)| (held.0, held.1)).collect();
        assert_eq!(held, [(1, 1), (2, 0)]);
        drop(views);
        assert_eq!(timelines.bytes.load(Ordering::Acquire), 2 * VIEW_BYTES);
        drop((other, current));
    }
}
