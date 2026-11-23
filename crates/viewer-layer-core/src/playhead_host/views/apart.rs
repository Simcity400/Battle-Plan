
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
    posted: Option<(Hour, u64, u64)>,
}

impl Apart {
    pub fn new(entry: &Arc<Entry>, branch: BranchId) -> io::Result<Self> {
        let store = Store::new(entry);
        let reader = entry.playhead().declare(to_declared(&for_the_makers(entry)), Some(mirror::rebuild(Arc::clone(&entry.pages))))?;
        let consumer = entry.playhead().consumer(reader);
        Ok(Self { entry: Arc::clone(entry), store, reader, consumer, mirrors: Arc::default(), branch, applied: None, posted: None })
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn drive(&mut self, hour: Hour) -> bool {
        let entry = Arc::clone(&self.entry);
        self.consumer.advance_blocking(hour);
        let reached = entry.playhead().wait_for(|| {
            self.take_to(hour, true);
            self.applied == Some((hour, entry.playhead().epoch())) && self.home()
        });
        if reached.is_ok() {
            if let Some((applied, epoch)) = self.applied {
                self.store.post(&self.mirrors, applied, self.branch, epoch, false);
                self.posted = Some((applied, epoch, self.store.demand()));
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
        self.posted = Some((hour, epoch, self.store.demand()));
        self.store.settled();
    }

    #[cfg(any(test, feature = "test-support"))]
    fn home(&self) -> bool {
        for_the_makers(&self.entry).keys().all(|&table| self.mirrors.get(replay_data::rows::TableId::ALL[table]).is_some())
    }

    fn take_to(&mut self, hour: Hour, blocking: bool) {
        let mut steps: Vec<Arc<Step>> = Vec::new();
        #[cfg(any(test, feature = "test-support"))]
        if blocking {
            self.consumer.take_blocking(hour, &mut steps);
        } else {
            self.consumer.take(hour, &mut steps);
        }
        #[cfg(not(any(test, feature = "test-support")))]
        {
            let _ = blocking;
            self.consumer.take(hour, &mut steps);
        }
        if steps.is_empty() {
            return;
        }
        self.store.settled();
        let Some(mirrors) = Arc::get_mut(&mut self.mirrors) else { return };
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
        }
    }

    fn step(&mut self, hour: Hour, demand: u64) {
        self.consumer.advance(hour);
        self.take_to(hour, false);
        if let Some((applied, epoch)) = self.applied
            && self.posted != Some((applied, epoch, demand))
        {
            self.store.post(&self.mirrors, applied, self.branch, epoch, false);
            self.posted = Some((applied, epoch, demand));
        }
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

impl Following {
    pub fn open(entry: &Arc<Entry>, branch: BranchId) -> io::Result<Self> {
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
                loop {
                    let playhead = entry.playhead();
                    let activity = playhead.activity();
                    let demand = apart.store().demand();
                    let has_readers = apart.store().has_readers();
                    if has_readers {
                        apart.step(target_hour(playhead, Instant::now()), demand);
                    }
                    let mut stopping = stop.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
                    if *stopping {
                        return;
                    }
                    if apart.store().demand() != demand || playhead.activity() != activity {
                        continue;
                    }
                    if has_readers && playhead.playing() {
                        let tick = pace_period(playhead);
                        stopping = stop.1.wait_timeout(stopping, tick).unwrap_or_else(std::sync::PoisonError::into_inner).0;
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
            tests::{COUNTRIES, LIMIT, RECORDING, hourly_steps, record},
        },
    };
    use std::time::Duration;

    #[test]
    fn paused_late_reader_receives_its_first_frame_without_clock_advancing() {
        let entry = open_clocked(record("paused late apart reader", 16), hourly_steps(), 0, Side::Follow).unwrap();
        entry.set_capture(replay_data::scope::CaptureMode::Full);
        entry.playhead().seek(1);
        let following = Following::open(&entry, RECORDING.branch).unwrap();
        let first = following.views().reader_at("first", &[Need::every(COUNTRIES)], Pace::Step);
        let frame = first.wait(|frame| frame.hour == 1, LIMIT).expect("first reader receives paused hour 1");
        assert_eq!(frame.hour, 1);

        let late = following.views().reader_at("late", &[Need::every(COUNTRIES)], Pace::Step);
        let late_frame = late.wait(|frame| frame.hour == 1, LIMIT).expect("late reader receives paused hour 1 without clock advancing");
        assert_eq!(late_frame.hour, 1);

        let deferred = following.views().reader_later("deferred", &[Need::every(COUNTRIES)], Pace::Step);
        let deferred_frame = deferred.wait(|frame| frame.hour == 1, LIMIT).expect("deferred reader receives paused hour 1 without clock advancing");
        assert_eq!(deferred_frame.hour, 1);
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
