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
}

impl Default for Timelines {
    fn default() -> Self {
        Self::with_budget(KEPT_BYTES)
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
        }
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
        }
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
    changes: Vec<Change>,
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
            for change in &day.changes {
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
            changes,
        });
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
