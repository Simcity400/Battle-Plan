
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use replay_core::WorldFrame;

use crate::timeline::OwnerProvider;

#[derive(Clone)]
pub struct DisplayedHour {
    pub hour: i64,
    pub frame: Option<Arc<WorldFrame>>,
    pub owners: Arc<BTreeMap<u32, String>>,
    pub controllers: Arc<BTreeMap<u32, String>>,
    pub label: String,
    pub anomaly_note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScrubStatus {
    Settled,
    Converging { requested: i64 },
    Refused { requested: i64, reason: String },
}

enum Answer {
    Ready(Box<DisplayedHour>),
    Refused { hour: i64, reason: String },
}

struct Wanted {
    hour: Option<i64>,
    finished: Option<(u64, Answer)>,
    generation: u64,
    shutting_down: bool,
}

struct Shared {
    wanted: Mutex<Wanted>,
    wake: Condvar,
    live_generation: AtomicU64,
    worker_thread: std::sync::OnceLock<std::thread::ThreadId>,
    scanned_through: AtomicI64,
    eager_stalled_at: AtomicI64,
    park_entries: AtomicU64,
    stopped: AtomicBool,
}

pub struct ScrubEngine {
    shared: Arc<Shared>,
    provider: Arc<dyn OwnerProvider>,
    worker: Option<std::thread::JoinHandle<()>>,
    shown: DisplayedHour,
    status: ScrubStatus,
    shown_generation: u64,
}

fn resolve(provider: &dyn OwnerProvider, hour: i64) -> Result<DisplayedHour, String> {
    let frame = provider.world_frame_at(hour).map_err(|error| error.to_string())?;
    let owners = provider.owners_at(hour).map_err(|error| error.to_string())?;
    let controllers = provider
        .controllers_at(hour)
        .map_err(|error| error.to_string())?;
    let label = provider.hour_label(hour).map_err(|error| error.to_string())?;
    let anomaly_note = provider
        .anomaly_note(hour)
        .map_err(|error| error.to_string())?;
    Ok(DisplayedHour {
        hour,
        frame,
        owners,
        controllers,
        label,
        anomaly_note,
    })
}

impl ScrubEngine {
    pub fn start(
        provider: Arc<dyn OwnerProvider>,
        first_hour: i64,
    ) -> Result<ScrubEngine, String> {
        let first = resolve(provider.as_ref(), first_hour)?;
        let shared = Arc::new(Shared {
            wanted: Mutex::new(Wanted {
                hour: None,
                finished: None,
                generation: 0,
                shutting_down: false,
            }),
            wake: Condvar::new(),
            live_generation: AtomicU64::new(0),
            worker_thread: std::sync::OnceLock::new(),
            scanned_through: AtomicI64::new(first_hour),
            eager_stalled_at: AtomicI64::new(i64::MIN),
            park_entries: AtomicU64::new(0),
            stopped: AtomicBool::new(false),
        });
        let serving = Arc::new(AtomicU64::new(0));
        let switch_shared = Arc::clone(&shared);
        let switch_serving = Arc::clone(&serving);
        provider.set_scrub_interrupt(Some(Arc::new(move || {
            let Some(worker) = switch_shared.worker_thread.get() else {
                return true;
            };
            if std::thread::current().id() != *worker {
                return true;
            }
            switch_shared.live_generation.load(Ordering::SeqCst)
                == switch_serving.load(Ordering::SeqCst)
                && !switch_shared.stopped.load(Ordering::SeqCst)
        })));
        let worker_shared = Arc::clone(&shared);
        let worker_provider = Arc::clone(&provider);
        let worker_serving = Arc::clone(&serving);
        let worker = std::thread::Builder::new()
            .name("replay-scrub".to_string())
            .spawn(move || {
                let _ = worker_shared.worker_thread.set(std::thread::current().id());
                run_worker(worker_shared, worker_provider, worker_serving)
            })
            .map_err(|error| {
                provider.set_scrub_interrupt(None);
                format!("start the replay scrub worker: {error}")
            })?;
        Ok(ScrubEngine {
            shared,
            provider,
            worker: Some(worker),
            shown: first,
            status: ScrubStatus::Settled,
            shown_generation: 0,
        })
    }

    pub fn show(&mut self, requested: i64) -> (DisplayedHour, ScrubStatus) {
        if requested != self.shown.hour || !matches!(self.status, ScrubStatus::Settled) {
            self.post(requested);
        }
        self.collect(requested);
        (self.shown.clone(), self.status.clone())
    }

    pub fn shown(&self) -> &DisplayedHour {
        &self.shown
    }

    fn post(&mut self, requested: i64) {
        if let ScrubStatus::Converging { requested: in_flight } = &self.status {
            if *in_flight == requested {
                return;
            }
        }
        if let ScrubStatus::Refused { requested: refused, .. } = &self.status {
            if *refused == requested {
                return;
            }
        }
        let Ok(mut wanted) = self.shared.wanted.lock() else {
            return;
        };
        wanted.hour = Some(requested);
        wanted.generation += 1;
        self.shared
            .live_generation
            .store(wanted.generation, Ordering::SeqCst);
        self.status = ScrubStatus::Converging { requested };
        self.shared.wake.notify_all();
    }

    fn collect(&mut self, requested: i64) {
        let Ok(mut wanted) = self.shared.wanted.lock() else {
            return;
        };
        let current_generation = wanted.generation;
        let Some((generation, answer)) = wanted.finished.take() else {
            drop(wanted);
            self.notice_a_dead_worker(requested);
            return;
        };
        drop(wanted);
        let superseded = generation != current_generation;
        if superseded && matches!(answer, Answer::Refused { .. }) {
            return;
        }
        if generation < self.shown_generation {
            return;
        }
        self.shown_generation = generation;
        match answer {
            Answer::Ready(hour) => {
                let settled = hour.hour == requested && !superseded;
                self.shown = *hour;
                self.status = if settled {
                    ScrubStatus::Settled
                } else {
                    ScrubStatus::Converging { requested }
                };
            }
            Answer::Refused { hour, reason } => {
                self.status = ScrubStatus::Refused {
                    requested: hour,
                    reason,
                };
            }
        }
    }

    fn notice_a_dead_worker(&mut self, requested: i64) {
        if !matches!(self.status, ScrubStatus::Converging { .. }) {
            return;
        }
        let dead = self
            .worker
            .as_ref()
            .is_none_or(std::thread::JoinHandle::is_finished);
        if dead {
            self.status = ScrubStatus::Refused {
                requested,
                reason: "the replay scrub worker stopped; no further hour can be prepared"
                    .to_string(),
            };
        }
    }

    pub fn scanned_through(&self) -> i64 {
        self.shared.scanned_through.load(Ordering::Relaxed)
    }

    pub fn eager_stalled_hour(&self) -> Option<i64> {
        let stalled = self.shared.eager_stalled_at.load(Ordering::Relaxed);
        if stalled == i64::MIN {
            None
        } else {
            Some(stalled)
        }
    }

    pub fn worker_park_entries(&self) -> u64 {
        self.shared.park_entries.load(Ordering::SeqCst)
    }
}

impl Drop for ScrubEngine {
    fn drop(&mut self) {
        if let Ok(mut wanted) = self.shared.wanted.lock() {
            wanted.shutting_down = true;
        }
        self.shared.stopped.store(true, Ordering::SeqCst);
        self.shared.wake.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.provider.set_scrub_interrupt(None);
    }
}

fn run_worker(shared: Arc<Shared>, provider: Arc<dyn OwnerProvider>, serving: Arc<AtomicU64>) {
    let hours: Vec<i64> = provider.hours().to_vec();
    loop {
        let (hour, generation, eager) = {
            let Ok(mut wanted) = shared.wanted.lock() else {
                return;
            };
            loop {
                if wanted.shutting_down {
                    return;
                }
                if let Some(hour) = wanted.hour.take() {
                    break (hour, wanted.generation, false);
                }
                let frontier = shared.scanned_through.load(Ordering::Relaxed);
                let stalled = shared.eager_stalled_at.load(Ordering::Relaxed);
                let next = hours
                    .get(hours.partition_point(|hour| *hour <= frontier))
                    .copied();
                match next {
                    Some(next) if next != stalled => break (next, wanted.generation, true),
                    _ => {}
                }
                shared.park_entries.fetch_add(1, Ordering::SeqCst);
                let Ok(next) = shared.wake.wait(wanted) else {
                    return;
                };
                wanted = next;
            }
        };

        serving.store(generation, Ordering::SeqCst);
        let answer = resolve(provider.as_ref(), hour);
        if shared.stopped.load(Ordering::SeqCst) {
            return;
        }
        if eager {
            match answer {
                Ok(_) => shared.scanned_through.store(hour, Ordering::Relaxed),
                Err(_) => {
                    if shared.live_generation.load(Ordering::SeqCst) == generation {
                        shared.eager_stalled_at.store(hour, Ordering::Relaxed);
                    }
                }
            }
            continue;
        }

        if answer.is_ok() {
            shared.scanned_through.fetch_max(hour, Ordering::Relaxed);
        }
        let Ok(mut wanted) = shared.wanted.lock() else {
            return;
        };
        if wanted.generation != generation {
            continue;
        }
        wanted.finished = Some((
            generation,
            match answer {
                Ok(displayed) => Answer::Ready(Box::new(displayed)),
                Err(reason) => Answer::Refused { hour, reason },
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize};
    use std::time::{Duration, Instant};

    struct SlowProvider {
        hours: Vec<i64>,
        per_hour: Duration,
        resolved: AtomicUsize,
        refusals: AtomicUsize,
        refuse: Option<i64>,
        panic_at: Option<i64>,
    }

    impl SlowProvider {
        fn new(count: i64, per_hour: Duration, refuse: Option<i64>) -> Arc<SlowProvider> {
            Arc::new(SlowProvider {
                hours: (0..count).collect(),
                per_hour,
                resolved: AtomicUsize::new(0),
                refusals: AtomicUsize::new(0),
                refuse,
                panic_at: None,
            })
        }

        fn panicking_at(count: i64, hour: i64) -> Arc<SlowProvider> {
            Arc::new(SlowProvider {
                hours: (0..count).collect(),
                per_hour: Duration::from_millis(1),
                resolved: AtomicUsize::new(0),
                refusals: AtomicUsize::new(0),
                refuse: None,
                panic_at: Some(hour),
            })
        }
    }

    impl OwnerProvider for SlowProvider {
        fn hours(&self) -> &[i64] {
            &self.hours
        }
        fn owners_at(&self, hour: i64) -> crate::Result<Arc<BTreeMap<u32, String>>> {
            if self.panic_at == Some(hour) {
                panic!("provider stack died at hour {hour}");
            }
            self.resolved.fetch_add(1, Ordering::SeqCst);
            if self.refuse == Some(hour) {
                self.refusals.fetch_add(1, Ordering::SeqCst);
                return Err(format!("hour {hour} is inside a declared coverage gap").into());
            }
            std::thread::sleep(self.per_hour);
            Ok(Arc::new(BTreeMap::from([(1u32, format!("H{hour}"))])))
        }
        fn controllers_at(&self, hour: i64) -> crate::Result<Arc<BTreeMap<u32, String>>> {
            self.owners_at(hour)
        }
        fn hour_label(&self, hour: i64) -> crate::Result<String> {
            Ok(format!("hour {hour}"))
        }
    }

    #[test]
    fn every_link_that_carries_the_stop_switch_to_a_reader_delegates() {
        let links: [(&str, &str, &str); 4] = [
            (
                include_str!("timeline.rs"),
                "OwnerProvider for RecordingTimeline",
                "self.timeline.set_scrub_interrupt(",
            ),
            (
                include_str!("../../replay_core/src/world.rs"),
                "WorldTimeline",
                "self.source.set_scrub_interrupt(",
            ),
            (
                include_str!("../../replay_core/src/source.rs"),
                "ReplaySource for IndexedReplaySource",
                "self.cache.set_scrub_interrupt(",
            ),
            (
                include_str!("recording_open.rs"),
                "ReplaySource for ProductReplaySource",
                "self.inner.set_scrub_interrupt(",
            ),
        ];
        for (source, who, delegation) in links {
            let source = source.replace("
", "
");
            assert!(
                source.contains("fn set_scrub_interrupt("),
                "{who} must implement set_scrub_interrupt"
            );
            assert!(
                source.contains(delegation),
                "{who} must delegate the stop switch onward with {delegation}; without it the                  switch stops here and a scrub can never be interrupted below this point"
            );
        }
    }

    struct InterruptibleProvider {
        hours: Vec<i64>,
        switch: Mutex<Option<Arc<dyn Fn() -> bool + Send + Sync>>>,
        abandoned: AtomicBool,
        abandoned_hours: Mutex<Vec<i64>>,
        steps: usize,
    }

    impl OwnerProvider for InterruptibleProvider {
        fn hours(&self) -> &[i64] {
            &self.hours
        }
        fn set_scrub_interrupt(&self, interrupt: Option<Arc<dyn Fn() -> bool + Send + Sync>>) {
            if let Ok(mut held) = self.switch.lock() {
                *held = interrupt;
            }
        }
        fn owners_at(&self, hour: i64) -> crate::Result<Arc<BTreeMap<u32, String>>> {
            let switch = self.switch.lock().ok().and_then(|held| held.clone());
            for _ in 0..self.steps {
                std::thread::sleep(Duration::from_millis(1));
                if let Some(switch) = &switch {
                    if !switch() {
                        self.abandoned.store(true, Ordering::SeqCst);
                        if let Ok(mut seen) = self.abandoned_hours.lock() {
                            seen.push(hour);
                        }
                        return Err(format!(
                            "the reconstruction of hour {hour} was abandoned because the                              caller's stop switch asked it to stop"
                        )
                        .into());
                    }
                }
            }
            Ok(Arc::new(BTreeMap::from([(1u32, format!("H{hour}"))])))
        }
        fn controllers_at(&self, hour: i64) -> crate::Result<Arc<BTreeMap<u32, String>>> {
            self.owners_at(hour)
        }
        fn hour_label(&self, hour: i64) -> crate::Result<String> {
            Ok(format!("hour {hour}"))
        }
    }

    #[test]
    fn a_seek_the_user_dragged_past_is_told_to_stop() {
        let provider = Arc::new(InterruptibleProvider {
            hours: (0..200).collect(),
            switch: Mutex::new(None),
            abandoned: AtomicBool::new(false),
            abandoned_hours: Mutex::new(Vec::new()),
            steps: 400,
        });
        let watcher = Arc::clone(&provider);
        let mut engine = ScrubEngine::start(provider, 0).unwrap();

        let deadline = Instant::now() + Duration::from_secs(20);
        let mut hour = 1;
        while !watcher.abandoned.load(Ordering::SeqCst) {
            engine.show(hour);
            hour = if hour >= 190 { 1 } else { hour + 1 };
            assert!(
                Instant::now() < deadline,
                "no reconstruction was ever told to stop; the stop switch is not reaching the                  provider"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn dropping_the_engine_gives_the_provider_its_switch_back() {
        let provider = Arc::new(InterruptibleProvider {
            hours: (0..20).collect(),
            switch: Mutex::new(None),
            abandoned: AtomicBool::new(false),
            abandoned_hours: Mutex::new(Vec::new()),
            steps: 1,
        });
        let watcher = Arc::clone(&provider);
        assert!(
            watcher.switch.lock().unwrap().is_none(),
            "a provider starts with no switch"
        );
        {
            let mut engine = ScrubEngine::start(provider, 0).unwrap();
            engine.show(1);
            assert!(
                watcher.switch.lock().unwrap().is_some(),
                "a running engine must have installed its switch"
            );
        }
        assert!(
            watcher.switch.lock().unwrap().is_none(),
            "dropping the engine must uninstall the switch it installed"
        );
    }

    #[test]
    fn the_failed_spawn_path_gives_the_switch_back_too() {
        let source = include_str!("scrub.rs").replace("
", "
");
        let start = source
            .find("let worker = std::thread::Builder::new()")
            .expect("the spawn must exist; this test's anchor moved");
        let spawn_block = &source[start..];
        let end = spawn_block
            .find("Ok(ScrubEngine {")
            .expect("the spawn block must end before the constructor");
        let spawn_block = &spawn_block[..end];
        assert!(
            spawn_block.contains("provider.set_scrub_interrupt(None)"),
            "a spawn that fails leaves no ScrubEngine and therefore no Drop, so it must hand              the switch back itself or a retained provider keeps the closure for ever"
        );
    }

    #[test]
    fn a_refusal_the_timeline_has_already_moved_past_is_not_shown() {
        const REFUSED: i64 = 7;
        let provider = SlowProvider::new(60, Duration::from_millis(1), Some(REFUSED));
        let counter = Arc::clone(&provider);
        let mut engine = ScrubEngine::start(provider, 0).unwrap();

        engine.show(REFUSED);
        let deadline = Instant::now() + Duration::from_secs(10);
        while counter.refusals.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < deadline, "the refusing hour was never attempted");
            std::thread::yield_now();
        }
        std::thread::sleep(Duration::from_millis(100));

        let (_, status) = engine.show(30);
        assert_ne!(
            status,
            ScrubStatus::Refused {
                requested: REFUSED,
                reason: "hour 7 is inside a declared coverage gap".to_string()
            },
            "a refusal for an hour the timeline has moved past must not be adopted"
        );
        assert!(
            !matches!(status, ScrubStatus::Refused { .. }),
            "nothing about hour 30 was refused, but the status says {status:?}"
        );
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let (shown, status) = engine.show(30);
            if status == ScrubStatus::Settled {
                assert_eq!(shown.hour, 30);
                break;
            }
            assert!(Instant::now() < deadline, "never converged past the stale refusal");
            std::thread::yield_now();
        }
    }

    #[test]
    fn an_interrupted_eager_step_is_not_a_refusal() {
        let provider = Arc::new(InterruptibleProvider {
            hours: (0..200).collect(),
            switch: Mutex::new(None),
            abandoned: AtomicBool::new(false),
            abandoned_hours: Mutex::new(Vec::new()),
            steps: 60,
        });
        let watcher = Arc::clone(&provider);
        let mut engine = ScrubEngine::start(provider, 0).unwrap();

        let requested: Vec<i64> = (0..12).map(|step| 150 + step * 2).collect();
        for hour in &requested {
            let deadline = Instant::now() + Duration::from_secs(30);
            while engine.show(*hour).1 != ScrubStatus::Settled {
                assert!(Instant::now() < deadline, "never settled on hour {hour}");
                std::thread::sleep(Duration::from_millis(2));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let abandoned_eager = watcher
            .abandoned_hours
            .lock()
            .map(|seen| seen.iter().any(|hour| !requested.contains(hour)))
            .unwrap_or(false);
        assert!(
            abandoned_eager,
            "no EAGER step was ever interrupted, so this test proves nothing about the eager              rule; abandoned hours were {:?}",
            watcher.abandoned_hours.lock().map(|seen| seen.clone())
        );

        let parked_at = engine.scanned_through();
        engine.show(1);
        let deadline = Instant::now() + Duration::from_secs(60);
        while engine.scanned_through() <= parked_at {
            engine.show(1);
            assert!(
                Instant::now() < deadline,
                "the eager walk never advanced past {parked_at}; an interrupted step was                  treated as a refusal and parked the ordered scan"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn dragging_the_timeline_never_waits_for_the_hour_it_asked_for() {
        let provider = SlowProvider::new(200, Duration::from_millis(50), None);
        let mut engine = ScrubEngine::start(provider, 0).unwrap();

        let start = Instant::now();
        for hour in 1..=40 {
            let (shown, status) = engine.show(hour);
            assert!(
                shown.hour >= 0 && shown.hour < 200,
                "a scrub must always have a real recorded hour to draw"
            );
            if shown.hour != hour {
                assert_eq!(status, ScrubStatus::Converging { requested: hour });
            }
        }
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(50),
            "40 drag positions took {elapsed:?}; the drawing thread is waiting on the worker"
        );
    }

    #[test]
    fn a_settled_request_converges_to_the_hour_that_was_asked_for() {
        let provider = SlowProvider::new(50, Duration::from_millis(5), None);
        let mut engine = ScrubEngine::start(provider, 0).unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let (shown, status) = engine.show(30);
            if status == ScrubStatus::Settled {
                assert_eq!(shown.hour, 30);
                assert_eq!(shown.label, "hour 30");
                break;
            }
            assert!(Instant::now() < deadline, "the scrub never converged on hour 30");
            std::thread::yield_now();
        }
    }

    #[test]
    fn a_refused_hour_is_named_and_stops_the_timeline_claiming_to_converge() {
        let provider = SlowProvider::new(50, Duration::from_millis(1), Some(20));
        let mut engine = ScrubEngine::start(provider, 0).unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let (shown, status) = engine.show(20);
            if let ScrubStatus::Refused { requested, reason } = &status {
                assert_eq!(*requested, 20);
                assert!(
                    reason.contains("declared coverage gap"),
                    "the refusal must carry the recording's own reason, got {reason:?}"
                );
                assert_ne!(shown.hour, 20);
                break;
            }
            assert!(
                Instant::now() < deadline,
                "a refused hour was never reported; status stayed {status:?}"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn a_dead_worker_is_reported_instead_of_seeking_for_ever() {
        let provider = SlowProvider::panicking_at(50, 30);
        let mut engine = ScrubEngine::start(provider, 0).unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let (shown, status) = engine.show(30);
            if let ScrubStatus::Refused { requested, reason } = &status {
                assert_eq!(*requested, 30);
                assert!(
                    reason.contains("scrub worker stopped"),
                    "a dead worker must be named as such, got {reason:?}"
                );
                assert_eq!(shown.hour, 0, "the last good hour stays on screen");
                assert_ne!(shown.hour, 30);
                break;
            }
            assert!(
                Instant::now() < deadline,
                "a dead worker was never reported; status stayed {status:?}"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn the_eager_walk_does_not_retry_the_hour_it_was_refused() {
        let provider = SlowProvider::new(50, Duration::from_millis(1), Some(1));
        let counter = Arc::clone(&provider);
        let mut engine = ScrubEngine::start(provider, 0).unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        while engine.show(0).1 != ScrubStatus::Settled {
            assert!(Instant::now() < deadline, "engine never settled on hour 0");
            std::thread::yield_now();
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while counter.refusals.load(Ordering::SeqCst) == 0 {
            assert!(
                Instant::now() < deadline,
                "hour 1 was never attempted by the eager walk"
            );
            std::thread::yield_now();
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while engine.eager_stalled_hour() != Some(1) || engine.worker_park_entries() == 0 {
            assert!(
                Instant::now() < deadline,
                "the refusal of hour 1 never parked the eager walk behind the stall"
            );
            std::thread::yield_now();
        }
        let resolved_at_stall = counter.resolved.load(Ordering::SeqCst);
        for _ in 0..20 {
            std::thread::sleep(Duration::from_millis(10));
            assert_eq!(
                counter.refusals.load(Ordering::SeqCst),
                1,
                "the eager walk attempted hour 1 more than once for its refusal"
            );
            assert_eq!(
                counter.resolved.load(Ordering::SeqCst),
                resolved_at_stall,
                "the eager walk kept resolving hours after it was stalled"
            );
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        while engine.show(2).1 != ScrubStatus::Settled {
            assert!(
                Instant::now() < deadline,
                "the scrub never settled on hour 2 after the refusal stalled the walk"
            );
            std::thread::yield_now();
        }
        assert_eq!(
            counter.refusals.load(Ordering::SeqCst),
            1,
            "hour 1 was attempted again after the walk moved past it"
        );
    }

    #[test]
    fn an_idle_timeline_walks_ahead_on_its_own() {
        let provider = SlowProvider::new(100, Duration::from_millis(1), None);
        let engine = ScrubEngine::start(provider, 0).unwrap();

        let deadline = Instant::now() + Duration::from_secs(10);
        while engine.scanned_through() < 20 {
            assert!(
                Instant::now() < deadline,
                "the eager walk did not advance; frontier stuck at {}",
                engine.scanned_through()
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn a_long_drag_resolves_far_fewer_hours_than_it_requests() {
        let provider = SlowProvider::new(400, Duration::from_millis(5), None);
        let counter = Arc::clone(&provider);
        let mut engine = ScrubEngine::start(provider, 0).unwrap();

        for hour in 1..=300 {
            engine.show(hour);
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        while engine.show(300).1 != ScrubStatus::Settled {
            assert!(Instant::now() < deadline, "the drag never converged on hour 300");
            std::thread::yield_now();
        }
        std::thread::sleep(Duration::from_millis(200));
        let resolved = counter.resolved.load(Ordering::SeqCst) / 2;
        assert!(
            resolved < 60,
            "a 300-position drag resolved {resolved} hours; requests are being queued rather \
             than superseded"
        );
    }
}
