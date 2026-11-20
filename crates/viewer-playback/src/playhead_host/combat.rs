//! The open combat window's read, made by the map's side for each publication's own hour. The
//! frame posts what the window reads ([`Asking`]) with the view; the step asks its maker at the
//! apply for the read of the hour it applied, over the side's own rows of that hour, and joins it at
//! its end, so the read reaches the frame in the publication it is of and the window stands at the
//! map's hour. A read that misses its step is never handed with a later one.
//!
//! A step waits for the read while the head stands still or a saved frame asks, and while the
//! hours run only while the last read fitted one publication's interval: past that the window
//! says it is loading at the top speeds rather than slow the map down.

use super::views;
use crate::layers::Settling;
use replay_data::{BranchId, Hour, rows::TableId};
use replay_frame::frame::DecodedFrame;
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use viewer_combat::reads::Reads;
use viewer_foundation::engagement_history::History;

/// How long a step that waits for the read waits, and a read for its rows: a saved frame shows
/// exactly its hour or names what it waited for.
const WHOLE: Duration = Duration::from_secs(120);

/// How long a read waiting for its rows sleeps between looks at whether a newer ask passed it over.
const LOOK: Duration = Duration::from_millis(2);

/// What the side reads an open window part at, posted by the frame: everything but the rows.
pub trait Asking: Clone + Send + 'static {
    /// Which window and settings a read is of.
    type Key: Clone + PartialEq + std::fmt::Debug + Send + 'static;
    type Made: Send + 'static;
    fn key(&self) -> Self::Key;
    /// The read of `rows`, of `hour`, made at the side's `seek`.
    fn read(&self, branch: BranchId, hour: Hour, seek: u64, rows: Arc<DecodedFrame>, engaged: Option<Arc<History>>) -> Self::Made;
}

impl Asking for viewer_combat::Asking {
    type Key = viewer_combat::ReadKey;
    type Made = viewer_combat::Product;

    fn key(&self) -> Self::Key {
        viewer_combat::Asking::key(self)
    }

    fn read(&self, branch: BranchId, hour: Hour, seek: u64, rows: Arc<DecodedFrame>, engaged: Option<Arc<History>>) -> Self::Made {
        viewer_combat::Asking::read(self, branch, hour, seek, rows, engaged)
    }
}

/// What a [`Combat::join`] did: whether its ask was answered, and whether it took the read asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Joined {
    pub answered: bool,
    pub made: bool,
}

/// One read: of this picture (`generation`), at this hour, under this engaged cover and asking.
#[derive(Clone, Debug, PartialEq)]
struct Key<K> {
    generation: u64,
    branch: BranchId,
    hour: Hour,
    covered: bool,
    read: K,
}

/// The maker: the worker the reads run on and the window's rows from the side's view store.
struct Maker<A: Asking> {
    reads: Reads<Key<A::Key>, Result<A::Made, String>>,
    rows: Arc<views::Reader>,
    declared: Vec<TableId>,
    /// The newest key asked, and how many keys have been: a read waiting for its rows gives up once
    /// another is asked.
    last: Option<Key<A::Key>>,
    newest: Arc<AtomicU64>,
    /// The last read's nanoseconds, its wait for rows included: whether it fits a publication.
    took: Arc<AtomicU64>,
}

/// The open combat window's read as the side's step holds it.
pub struct Combat<A: Asking> {
    asking: Option<A>,
    maker: Option<Maker<A>>,
    /// Tables a read found its rows did not hold, declared from then on, across windows.
    learned: Vec<TableId>,
    /// Bumped by every reset: nothing read for the picture before is handed as this one's.
    generation: u64,
    /// What this step asked, which its join takes.
    asked: Option<Key<A::Key>>,
    /// The read last handed to the frame: a step at its hour asks for nothing, since the frame holds
    /// it already.
    handed: Option<Key<A::Key>>,
    made: Option<A::Made>,
    loading: Vec<String>,
}

impl<A: Asking> Default for Combat<A> {
    fn default() -> Self {
        Self { asking: None, maker: None, learned: Vec::new(), generation: 0, asked: None, handed: None, made: None, loading: Vec::new() }
    }
}

impl<A: Asking> Combat<A> {
    /// The frame's newest asking; none closes the window, and its maker and rows with it.
    pub fn ask(&mut self, asking: Option<A>) {
        if asking.is_none() {
            (self.maker, self.asked, self.handed, self.made) = (None, None, None, None);
            self.loading.clear();
        }
        self.asking = asking;
    }

    /// A reset: the picture changes, so no read made for the one before is handed.
    pub fn reset(&mut self) {
        self.generation += 1;
        (self.asked, self.handed, self.made) = (None, None, None);
    }

    /// The step asks for the read of `applied`, its hour, at its apply, over the rows it posts at
    /// `epoch`, so the read is made while its layers settle. Whether anything was asked for.
    pub fn ask_step(&mut self, store: &views::Store, applied: Option<Hour>, branch: Option<BranchId>, epoch: u64, engaged: Option<Arc<History>>) -> bool {
        self.asked = None;
        let (Some(asking), Some(hour), Some(branch)) = (self.asking.clone(), applied, branch) else { return false };
        let covered = engaged.as_deref().is_some_and(|index| index.covers(branch, hour));
        let key = Key { generation: self.generation, branch, hour, covered, read: asking.key() };
        if self.handed.as_ref() == Some(&key) {
            return false;
        }
        self.loading.clear();
        let learned = &self.learned;
        let maker = self.maker.get_or_insert_with(|| Maker::open(store, learned));
        if maker.last.as_ref() != Some(&key) {
            maker.last = Some(key.clone());
            maker.newest.fetch_add(1, Ordering::AcqRel);
        }
        let (rows, newest, took) = (Arc::clone(&maker.rows), Arc::clone(&maker.newest), Arc::clone(&maker.took));
        let stamp = newest.load(Ordering::Acquire);
        maker.reads.ask(
            key.clone(),
            move || {
                let started = Instant::now();
                let made = rows_at(&rows, hour, epoch, &newest, stamp).map(|frame| asking.read(branch, hour, epoch, frame, engaged));
                took.store(started.elapsed().as_nanos() as u64, Ordering::Release);
                made
            },
            || {},
        );
        self.asked = Some(key);
        true
    }

    /// The read this step asked for, taken when it is made: waited for while `settling` waits, and
    /// while the hours run for one `interval` if the last read fitted it. Whether the ask was
    /// answered (false when the read found a table its rows did not hold, which is declared and
    /// read again at the next ask), and whether this join took the read it asked for.
    pub fn join(&mut self, settling: Settling, interval: Duration) -> Joined {
        let (Some(key), Some(maker)) = (self.asked.take(), self.maker.as_mut()) else { return Joined { answered: true, made: false } };
        let limit = if settling.waits() {
            WHOLE
        } else if Duration::from_nanos(maker.took.load(Ordering::Acquire)) <= interval {
            interval
        } else {
            Duration::ZERO
        };
        maker.reads.wait(|done| *done == key, limit);
        let missed = maker.reads.take_missed();
        let answered = missed.is_empty();
        if !answered {
            self.loading.push(format!("{missed:?}"));
            self.learned.extend(missed);
            self.learned.sort_unstable_by_key(|table| table.index());
            self.learned.dedup();
            maker.declare(&self.learned);
        }
        let mut made = false;
        match maker.reads.take(|done| *done == key) {
            Some((_, Ok(read))) => {
                (self.made, self.handed) = (Some(read), Some(key));
                made = true;
            }
            Some((_, Err(why))) => self.loading.push(why),
            None if answered => self.loading.push(format!("its read of hour {}", key.hour)),
            None => {}
        }
        Joined { answered, made }
    }

    /// The read the last join took; the frame holds it from then on.
    pub fn take(&mut self) -> Option<A::Made> {
        self.made.take()
    }

    /// What the window is waiting for, by name; empty once a read is handed.
    pub fn loading(&self) -> &[String] {
        &self.loading
    }

    /// Whether a window is open to read.
    pub fn open(&self) -> bool {
        self.asking.is_some()
    }

    /// How many tables reads have found their rows did not hold.
    pub fn learned(&self) -> usize {
        self.learned.len()
    }
}

impl<A: Asking> Maker<A> {
    /// The worker and the window's rows, declared with the vision's tables and every table learned.
    fn open(store: &views::Store, learned: &[TableId]) -> Self {
        let declared = Self::tables(learned);
        let rows = Arc::new(store.reader("combat window", &declared.iter().copied().map(views::Need::every).collect::<Vec<_>>()));
        Self { reads: Reads::new("combat window reads"), rows, declared, last: None, newest: Arc::default(), took: Arc::default() }
    }

    fn tables(learned: &[TableId]) -> Vec<TableId> {
        let mut tables: Vec<TableId> = viewer_foundation::map::perspective::VISION_TABLES.iter().chain(learned).copied().collect();
        tables.sort_unstable_by_key(|table| table.index());
        tables.dedup();
        tables
    }

    /// Every table learned declared to the rows, which are made from the side's mirrors at once.
    fn declare(&mut self, learned: &[TableId]) {
        let tables = Self::tables(learned);
        if tables != self.declared {
            self.rows.declare_only(&tables);
            self.declared = tables;
        }
    }
}

impl<A: Asking> Drop for Maker<A> {
    /// A read waiting for its rows gives up, so the rows' reader closes with the window.
    fn drop(&mut self) {
        self.newest.fetch_add(1, Ordering::AcqRel);
    }
}

/// The window's rows of exactly `hour` as the step posted them at `epoch`: rows of another seek,
/// even of the same hour, are another history's. Refused by name once a newer ask (`newest` past
/// `stamp`) or the window's closing passes this one over, once the rows have passed the hour, or
/// after [`WHOLE`].
fn rows_at(rows: &views::Reader, hour: Hour, epoch: u64, newest: &AtomicU64, stamp: u64) -> Result<Arc<DecodedFrame>, String> {
    let until = Instant::now() + WHOLE;
    loop {
        let seen = rows.published();
        if let Some((frame, _, seek)) = rows.newest_seek()
            && seek == epoch
        {
            if frame.hour == hour {
                return Ok(frame);
            }
            if frame.hour > hour {
                return Err(format!("its rows of hour {hour}, which the side's steps have passed"));
            }
        }
        if newest.load(Ordering::Acquire) != stamp {
            return Err(format!("its rows of hour {hour}, passed over for a newer hour"));
        }
        if Instant::now() >= until {
            return Err(format!("its rows of hour {hour}, which did not come (waiting for {:?})", rows.arriving()));
        }
        rows.wait_newer(seen, LOOK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::playhead_host::{
        open,
        views::{
            apart::Apart,
            tests::{RECORDING, hourly_steps, record},
        },
    };
    use replay_data::scope::CaptureMode;
    use std::sync::{Mutex, mpsc};

    /// A window whose read is the hour of its rows, each read held until the test lets it finish.
    #[derive(Clone)]
    struct Held {
        started: Arc<Mutex<mpsc::Sender<Hour>>>,
        finish: Arc<Mutex<mpsc::Receiver<()>>>,
    }

    impl Asking for Held {
        type Key = ();
        type Made = Hour;

        fn key(&self) {}

        fn read(&self, _: BranchId, hour: Hour, _: u64, rows: Arc<DecodedFrame>, _: Option<Arc<History>>) -> Hour {
            assert_eq!(rows.hour, hour, "read over the rows of its own hour");
            let _ = self.started.lock().unwrap().send(hour);
            self.finish.lock().unwrap().recv_timeout(WHOLE).expect("the test lets the read finish");
            rows.hour
        }
    }

    /// Each step's read is of that step's hour: one that misses its step is never handed with a
    /// later step, which hands its own hour's read or nothing.
    #[test]
    fn the_read_handed_with_a_publication_is_of_its_hour_and_a_late_one_never_is() {
        let entry = open(record("combat window publication", 8), hourly_steps()).unwrap();
        entry.set_capture(CaptureMode::Full);
        entry.playhead().seek(1);
        let mut fed = Apart::new(&entry, RECORDING.branch).unwrap();
        let (started, reads) = mpsc::channel();
        let (finish, finishing) = mpsc::channel();
        let mut combat: Combat<Held> = Combat::default();
        combat.ask(Some(Held { started: Arc::new(Mutex::new(started)), finish: Arc::new(Mutex::new(finishing)) }));
        let branch = Some(RECORDING.branch);
        let epoch = entry.playhead().epoch();
        let read = || reads.recv_timeout(WHOLE).expect("a read starts");
        let interval = Duration::from_millis(1);

        // Each step asks before it posts its rows, as the side's does.
        assert!(combat.ask_step(fed.store(), Some(1), branch, epoch, None));
        assert!(fed.drive(1));
        assert_eq!(read(), 1);
        finish.send(()).unwrap();
        assert!(combat.join(Settling::still(), interval).answered);
        assert_eq!(combat.take(), Some(1), "the head standing still waits for its hour's read");
        assert!(!combat.ask_step(fed.store(), Some(1), branch, epoch, None), "the frame holds hour 1's read: nothing is read again");

        assert!(combat.ask_step(fed.store(), Some(2), branch, epoch, None));
        assert!(fed.drive(2));
        assert_eq!(read(), 2);
        assert!(combat.join(Settling::step(), interval).answered);
        assert_eq!(combat.take(), None, "playing, the step does not wait for a read past its interval");
        assert_eq!(combat.loading(), ["its read of hour 2"], "and says what the window waits for");

        assert!(combat.ask_step(fed.store(), Some(3), branch, epoch, None));
        assert!(fed.drive(3));
        finish.send(()).unwrap();
        assert_eq!(read(), 3, "hour 3's read follows hour 2's");
        finish.send(()).unwrap();
        assert!(combat.join(Settling::still(), interval).answered);
        assert_eq!(combat.take(), Some(3), "hour 3's publication hands hour 3's read, never hour 2's");
    }

    /// A join reports the read it took itself: a product of an earlier step the frame has not taken
    /// yet never stands for a later ask whose read failed.
    #[test]
    fn a_join_reports_only_the_read_it_took() {
        let entry = open(record("combat window join", 8), hourly_steps()).unwrap();
        entry.set_capture(CaptureMode::Full);
        entry.playhead().seek(1);
        let mut fed = Apart::new(&entry, RECORDING.branch).unwrap();
        let (started, reads) = mpsc::channel();
        let (finish, finishing) = mpsc::channel();
        let mut combat: Combat<Held> = Combat::default();
        combat.ask(Some(Held { started: Arc::new(Mutex::new(started)), finish: Arc::new(Mutex::new(finishing)) }));
        let (branch, epoch) = (Some(RECORDING.branch), entry.playhead().epoch());
        assert!(combat.ask_step(fed.store(), Some(1), branch, epoch, None));
        assert!(fed.drive(1));
        assert_eq!(reads.recv_timeout(WHOLE), Ok(1));
        finish.send(()).unwrap();
        assert_eq!(combat.join(Settling::still(), Duration::ZERO), Joined { answered: true, made: true });
        assert!(combat.ask_step(fed.store(), Some(0), branch, epoch, None), "hour 1's read is not taken yet: hour 0 is asked");
        assert_eq!(combat.join(Settling::still(), Duration::ZERO), Joined { answered: true, made: false }, "hour 0's rows are passed: this join took nothing");
        assert_eq!(combat.take(), Some(1), "hour 1's read is still the frame's to take");
    }

    /// A seek back to the hour the rows stand at asks for that hour again: the rows the step posted
    /// before the seek are of the same hour but another history, and never answer the read.
    #[test]
    fn rows_of_the_seek_before_never_answer_a_read_of_the_same_hour() {
        let entry = open(record("combat window seek", 8), hourly_steps()).unwrap();
        entry.set_capture(CaptureMode::Full);
        entry.playhead().seek(1);
        let mut fed = Apart::new(&entry, RECORDING.branch).unwrap();
        let maker = Maker::<Held>::open(fed.store(), &[]);
        let rows = Arc::clone(&maker.rows);
        assert!(fed.drive(1));
        let before = entry.playhead().epoch();
        let (asked, newer) = (AtomicU64::new(0), AtomicU64::new(1));
        assert_eq!(rows_at(&rows, 1, before, &asked, 0).map(|frame| frame.hour), Ok(1), "the rows of hour 1 at the seek they were posted at");
        assert!(rows_at(&rows, 1, before + 1, &newer, 0).is_err(), "a read asked after the seek waits for its own rows, never taking these");
    }
}
