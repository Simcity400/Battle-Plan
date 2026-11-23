
use super::{Store, Views};
use crate::playhead_host::{Entry, for_the_makers, mirror, to_declared};
use playhead::{Step, Watcher};
use replay_data::{BranchId, Hour};
use std::{
    io,
    sync::{Arc, Condvar, Mutex},
    thread::JoinHandle,
    time::{Duration, Instant},
};

const MIN_TICK: Duration = Duration::from_millis(4);
const MAX_TICK: Duration = Duration::from_millis(50);
const MIN_LEAD: Duration = Duration::from_millis(32);
const MAX_LEAD: Duration = Duration::from_millis(64);

const THREAD: &str = "rows apart";

pub struct Apart {
    entry: Arc<Entry>,
    store: Store,
    reader: playhead::Reader,
    consumer: playhead::Consumer,
    mirrors: Arc<mirror::Mirrors>,
    branch: BranchId,
    applied: Option<(Hour, u64)>,
    mirror_gen: u64,
    posted: Option<(Hour, u64, u64, u64)>,
}

impl Apart {
    pub fn new(entry: &Arc<Entry>, branch: BranchId) -> io::Result<Self> {
        let store = Store::new(entry);
        let reader = entry.playhead().declare(to_declared(&for_the_makers(entry)), Some(mirror::rebuild(Arc::clone(&entry.pages))))?;
        let consumer = entry.playhead().consumer(reader);
        Ok(Self { entry: Arc::clone(entry), store, reader, consumer, mirrors: Arc::default(), branch, applied: None, mirror_gen: 0, posted: None })
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn drive(&mut self, hour: Hour) -> bool {
        let entry = Arc::clone(&self.entry);
        self.consumer.advance_blocking(hour);
        let reached = entry.playhead().wait_for(|| {
            let _ = self.take_to(hour, true);
            self.applied == Some((hour, entry.playhead().epoch())) && self.home()
        });
        if reached.is_ok() {
            if let Some((applied, epoch)) = self.applied {
                self.store.post(&self.mirrors, applied, self.branch, epoch, false);
                self.posted = Some((applied, epoch, self.store.demand(), self.mirror_gen));
            }
            self.store.settled();
        }
        reached.is_ok()
    }

    #[cfg(test)]
    pub fn post_at(&mut self, hour: Hour) {
        self.store.settled();
        let epoch = self.applied.map_or(0, |(_, epoch)| epoch);
        self.store.post(&self.mirrors, hour, self.branch, epoch, false);
        self.posted = Some((hour, epoch, self.store.demand(), self.mirror_gen));
        self.store.settled();
    }

    #[cfg(any(test, feature = "test-support"))]
    fn home(&self) -> bool {
        for_the_makers(&self.entry).keys().all(|&table| self.mirrors.get(replay_data::rows::TableId::ALL[table]).is_some())
    }

    fn take_to(&mut self, hour: Hour, blocking: bool) -> bool {
        let mut steps: Vec<Arc<Step>> = Vec::new();
        #[cfg(any(test, feature = "test-support"))]
        if blocking {
            self.consumer.take_blocking(hour, &mut steps);
        } else if self.consumer.try_take(hour, &mut steps).is_none() {
            return true;
        }
        #[cfg(not(any(test, feature = "test-support")))]
        {
            let _ = blocking;
            if self.consumer.try_take(hour, &mut steps).is_none() {
                return true;
            }
        }
        if steps.is_empty() {
            return false;
        }
        self.store.settled();
        let Some(mirrors) = Arc::get_mut(&mut self.mirrors) else { return false };
        let mut tables = Vec::new();
        for step in &steps {
            if step.last() > hour {
                continue;
            }
            for changes in step.tables() {
                if changes.reset()
                    && let Some(state) = self.consumer.state(step, changes.table())
                    && let Ok(mut rebuilt) = state.downcast::<mirror::Mirror>()
                {
                    rebuilt.set_capture(self.entry.capture.get().copied());
                    mirrors.insert(rebuilt);
                }
            }
            mirrors.apply_timed(step, &mut tables, &mut Default::default(), &mut self.store.noting(&[]));
            self.applied = Some((step.last(), step.epoch()));
            self.mirror_gen += 1;
        }
        false
    }

    fn step(&mut self, hour: Hour, demand: u64) -> bool {
        let advance_ok = self.consumer.try_advance(hour);
        let take_contended = self.take_to(hour, false);
        if let Some((applied, epoch)) = self.applied
            && self.posted != Some((applied, epoch, demand, self.mirror_gen))
        {
            self.store.post(&self.mirrors, applied, self.branch, epoch, false);
            self.posted = Some((applied, epoch, demand, self.mirror_gen));
        }
        !advance_ok || take_contended
    }
}

fn pace_period(playhead: &playhead::Playhead) -> Duration {
    playhead.frame_period().clamp(MIN_TICK, MAX_TICK)
}

fn lead_duration(playhead: &playhead::Playhead) -> Duration {
    pace_period(playhead).saturating_mul(3).clamp(MIN_LEAD, MAX_LEAD)
}

fn target_hour(playhead: &playhead::Playhead, now: Instant) -> Hour {
    let head = playhead.head(now);
    playhead.head_from(head, lead_duration(playhead)).unwrap_or(head)
}

impl Drop for Apart {
    fn drop(&mut self) {
        self.entry.playhead().retire(self.reader);
    }
}

pub struct Following {
    views: Views,
    stop: Arc<(Mutex<bool>, Condvar)>,
    _watch: Watcher,
    thread: Option<JoinHandle<()>>,
}

#[cfg(test)]
type BeforeWait = Box<dyn Fn(Option<(Hour, u64)>, u64) + Send>;

impl Following {
    pub fn open(entry: &Arc<Entry>, branch: BranchId) -> io::Result<Self> {
        #[cfg(test)]
        {
            Self::open_with_hook(entry, branch, None)
        }
        #[cfg(not(test))]
        {
            Self::open_inner(entry, branch)
        }
    }

    #[cfg(not(test))]
    fn open_inner(entry: &Arc<Entry>, branch: BranchId) -> io::Result<Self> {
        Self::spawn(entry, branch, ())
    }

    #[cfg(test)]
    fn open_with_hook(entry: &Arc<Entry>, branch: BranchId, before_wait: Option<BeforeWait>) -> io::Result<Self> {
        Self::spawn(entry, branch, before_wait)
    }

    #[cfg(not(test))]
    fn notify_before_wait(_: &(), _: Option<(Hour, u64)>, _: u64) {}

    #[cfg(test)]
    fn notify_before_wait(hook: &Option<BeforeWait>, applied: Option<(Hour, u64)>, demand: u64) {
        if let Some(hook) = hook {
            hook(applied, demand);
        }
    }

    fn spawn(
        entry: &Arc<Entry>,
        branch: BranchId,
        #[cfg(test)] before_wait: Option<BeforeWait>,
        #[cfg(not(test))] before_wait: (),
    ) -> io::Result<Self> {
        let mut apart = Apart::new(entry, branch)?;
        let views = apart.store().handle();
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let wake: Arc<dyn Fn() + Send + Sync> = {
            let stop = Arc::clone(&stop);
            Arc::new(move || {
                let _guard = stop.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                drop(_guard);
                stop.1.notify_all();
            })
        };
        apart.store().set_wake(Arc::clone(&wake));
        let watch = entry.playhead().watch(wake);
        let entry = Arc::clone(entry);
        let thread = std::thread::Builder::new().name(THREAD.into()).spawn({
            let stop = Arc::clone(&stop);
            move || {
                let mut next_tick: Option<Instant> = None;
                let mut last_control: Option<u64> = None;
                let mut last_demand: Option<u64> = None;
                let mut contended = false;
                loop {
                    let playhead = entry.playhead();
                    let control = playhead.control_seq();
                    let activity = playhead.activity();
                    let demand = apart.store().demand();
                    let has_readers = apart.store().has_readers();
                    let playing = playhead.playing();
                    let tick = pace_period(playhead);
                    let now = Instant::now();
                    let control_changed = last_control != Some(control);
                    let demand_changed = last_demand != Some(demand);
                    if has_readers {
                        let due = !playing || contended || control_changed || demand_changed || next_tick.is_none_or(|deadline| now >= deadline);
                        if due {
                            let target = target_hour(playhead, now);
                            contended = apart.step(target, demand);
                            last_control = Some(control);
                            last_demand = Some(demand);
                            next_tick = if playing { Some(now + tick) } else { None };
                        } else if !playing {
                            next_tick = None;
                        }
                    } else {
                        contended = false;
                        last_control = Some(control);
                        last_demand = Some(demand);
                        next_tick = None;
                    }
                    Self::notify_before_wait(&before_wait, apart.applied, demand);
                    let mut stopping = stop.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    if *stopping {
                        return;
                    }
                    if apart.store().demand() != demand
                        || playhead.control_seq() != control
                        || (!playing && playhead.activity() != activity)
                    {
                        continue;
                    }
                    if contended {
                        stopping = stop.1.wait_timeout(stopping, Duration::from_millis(2)).unwrap_or_else(std::sync::PoisonError::into_inner).0;
                    } else if let Some(deadline) = next_tick {
                        let wait = deadline.saturating_duration_since(Instant::now());
                        if !wait.is_zero() {
                            stopping = stop.1.wait_timeout(stopping, wait).unwrap_or_else(std::sync::PoisonError::into_inner).0;
                        }
                    } else {
                        stopping = stop.1.wait(stopping).unwrap_or_else(std::sync::PoisonError::into_inner);
                    }
                    if *stopping {
                        return;
                    }
                }
            }
        })?;
        Ok(Self { views, stop, _watch: watch, thread: Some(thread) })
    }

    pub fn views(&self) -> &Views {
        &self.views
    }
}

impl Drop for Following {
    fn drop(&mut self) {
        {
            let mut stopping = self.stop.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            *stopping = true;
        }
        self.stop.1.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Following, lead_duration, pace_period};
    use crate::playhead_host::{
        Side, open_clocked,
        views::{
            Need, Pace,
            tests::{COUNTRIES, RECORDING, hourly_steps, record},
        },
    };
    use std::{
        sync::{Arc, atomic::{AtomicBool, AtomicUsize, Ordering}, mpsc},
        time::Duration,
    };

    #[test]
    fn paused_late_reader_receives_its_first_frame_without_clock_advancing() {
        let entry = open_clocked(record("paused late apart reader", 16), hourly_steps(), 0, Side::Follow).unwrap();
        entry.set_capture(replay_data::scope::CaptureMode::Full);
        entry.playhead().seek(1);
        let (wait_tx, wait_rx) = mpsc::channel();
        let following = Following::open_with_hook(&entry, RECORDING.branch, Some(Box::new(move |applied, demand| {
            let _ = wait_tx.send((applied, demand));
        }))).unwrap();
        let first = following.views().reader_at("first", &[Need::every(COUNTRIES)], Pace::Step);
        let first_demand = following.views().demand();
        let epoch = entry.playhead().epoch();
        entry.playhead().wait_for(|| first.frame().is_some_and(|frame| frame.hour == 1)).unwrap();
        while wait_rx.recv().unwrap() != (Some((1, epoch)), first_demand) {}

        let late = following.views().reader_at("late", &[Need::every(COUNTRIES)], Pace::Step);
        let late_demand = following.views().demand();
        entry.playhead().wait_for(|| late.frame().is_some_and(|frame| frame.hour == 1)).unwrap();
        while wait_rx.recv().unwrap() != (Some((1, epoch)), late_demand) {}

        let deferred = following.views().reader_later("deferred", &[Need::every(COUNTRIES)], Pace::Step);
        entry.playhead().wait_for(|| deferred.frame().is_some_and(|frame| frame.hour == 1)).unwrap();
    }

    #[test]
    fn seeking_while_apart_enters_wait_never_deadlocks() {
        let entry = open_clocked(record("apart seek lock order", 16), hourly_steps(), 0, Side::Follow).unwrap();
        entry.set_capture(replay_data::scope::CaptureMode::Full);
        let clock_locked = Arc::new(AtomicBool::new(false));
        let watcher_calls = Arc::new(AtomicUsize::new(0));
        let _probe = entry.playhead().watch({
            let entry = Arc::clone(&entry);
            let clock_locked = Arc::clone(&clock_locked);
            let watcher_calls = Arc::clone(&watcher_calls);
            Arc::new(move || {
                watcher_calls.fetch_add(1, Ordering::Relaxed);
                if !entry.playhead().clock_is_unlocked() {
                    clock_locked.store(true, Ordering::Release);
                }
            })
        });
        entry.playhead().seek(1);
        entry.playhead().play(24.0);
        entry.playhead().pause();
        entry.playhead().place(3);
        entry.playhead().frame_rate(60.0);
        let following = Following::open(&entry, RECORDING.branch).unwrap();
        let reader = following.views().reader_at("reader", &[Need::every(COUNTRIES)], Pace::Step);
        entry.playhead().wait_for(|| reader.frame().is_some_and(|frame| frame.hour == 3)).unwrap();
        for step in 0..200 {
            entry.playhead().seek((step % 15) + 1);
        }
        entry.playhead().seek(15);
        entry.playhead().wait_for(|| reader.frame().is_some_and(|frame| frame.hour == 15)).unwrap();
        assert!(watcher_calls.load(Ordering::Acquire) > 0, "watcher must have run on playhead control transitions");
        assert!(!clock_locked.load(Ordering::Acquire), "playhead watcher ran while clock mutex was held");
    }

    #[test]
    fn apart_paces_from_playhead_frame_rate() {
        let entry = open_clocked(record("apart display rate", 16), hourly_steps(), 0, Side::Follow).unwrap();
        entry.playhead().frame_rate(60.0);
        assert!(pace_period(entry.playhead()) >= Duration::from_millis(16) && pace_period(entry.playhead()) <= Duration::from_millis(17));
        assert!(lead_duration(entry.playhead()) >= Duration::from_millis(48) && lead_duration(entry.playhead()) <= Duration::from_millis(52));

        entry.playhead().frame_rate(120.0);
        assert!(pace_period(entry.playhead()) >= Duration::from_millis(8) && pace_period(entry.playhead()) <= Duration::from_millis(9));
        assert_eq!(lead_duration(entry.playhead()), Duration::from_millis(32));

        entry.playhead().frame_rate(30.0);
        assert!(pace_period(entry.playhead()) >= Duration::from_millis(33) && pace_period(entry.playhead()) <= Duration::from_millis(34));
        assert_eq!(lead_duration(entry.playhead()), Duration::from_millis(64));
    }
}
