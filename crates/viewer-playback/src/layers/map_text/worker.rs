//! The map text layer's worker: makes the intervals that hold events, in the order they are sent,
//! each from its events' context and, for events that read a province's fog, the viewer's sight,
//! as the step that carries it left them. It holds them itself: a reader of those tables of its
//! own ([`Stepped`]), stepped in order exactly to
//! each interval's hour and no further than the frame side has handed every interval on
//! ([`Worker::through`]), so the rows of every step are there however many steps the map took in
//! one frame. A table that has not arrived (declared after a seek) holds the interval until a later
//! step the frame side has handed on brings it: the interval is then made from those rows, once.
//! A table the recording holds nothing of is refused by name. Nothing is given up for time. A reset
//! makes earlier sends stale; the thread ends when the layer drops.
//!
//! Every request names the playhead's seek it came from: the reader applies only that seek's
//! steps for it, and an interval of a seek since superseded is dropped unmade, its steps abandoned.

use super::{Presented, Source, worker_tables};
use crate::{layers::rebuilt::refusal, playhead_host::rows::Stepped};
use replay_data::{
    BranchId, Hour,
    rows::{TABLE_COUNT, TableId},
};
use replay_frame::frame::{DecodedFrame, Metrics, SharedView, TableView, not_declared};
use std::{
    collections::VecDeque,
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    time::{Duration, Instant},
};
use viewer_foundation::map::{
    memo::Keyed,
    perspective::{Choice, VISION_TABLES, VisionKey, Vision as Seen, cached_vision},
    visibility::FogContent,
};
use viewer_world::map::animated_map_text::events::Prepared;

/// How often an interval held for a table not arrived yet looks again for a step bringing it while
/// no message comes (a compose's settle waits on the worker): a poll, never a limit.
const LOOK: Duration = Duration::from_millis(50);

/// One interval holding events, as the frame side hands it over.
pub(super) struct Job {
    pub at: (BranchId, Hour),
    pub seek: bool,
    /// The playhead's seek of the step that carried it ([`playhead::Step::epoch`]).
    pub epoch: u64,
    pub interval: SharedView,
    /// The perspective its events are decided for, with the vision derived from its own hour's rows.
    pub choice: Choice,
}

enum Work {
    Make { serial: u64, job: Job },
    /// Every interval of seek `epoch` through `hour` has been handed on: the context may step there.
    Through { hour: Hour, epoch: u64 },
}

struct Sent {
    generation: u64,
    work: Work,
}

type Made = (u64, u64, Presented);

pub(super) struct Worker {
    jobs: Option<Sender<Sent>>,
    made: Receiver<Made>,
    generation: Arc<AtomicU64>,
    serial: u64,
    stop: Arc<AtomicBool>,
    #[cfg(test)]
    alive: Arc<AtomicBool>,
    /// Made intervals a wait received before they were taken.
    held: Vec<(u64, Presented)>,
}

impl Worker {
    /// The worker, reading the events' context through `stepped` (every interval's context refused
    /// by its name without it).
    pub(super) fn start(source: Arc<dyn Source>, branch: BranchId, stepped: Result<Stepped, String>) -> io::Result<Self> {
        let (jobs, received) = mpsc::channel::<Sent>();
        let (made, taken) = mpsc::channel::<Made>();
        let generation = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let alive = Arc::new(AtomicBool::new(true));
        let run = Run { source, branch, stepped, generation: Arc::clone(&generation), stop: Arc::clone(&stop), vision: Keyed::new(VISION_TABLES) };
        let living = Arc::clone(&alive);
        std::thread::Builder::new().name("map text".into()).spawn(move || {
            viewer_foundation::worker_priority::background_priority();
            run.run(&received, &made);
            living.store(false, Ordering::Release);
        })?;
        Ok(Self {
            jobs: Some(jobs),
            made: taken,
            generation,
            serial: 0,
            stop,
            #[cfg(test)]
            alive,
            held: Vec::new(),
        })
    }

    fn post(&self, work: Work) {
        let generation = self.generation.load(Ordering::Acquire);
        if let Some(jobs) = &self.jobs {
            let _ = jobs.send(Sent { generation, work });
        }
    }

    /// Sends `job`; its serial.
    pub(super) fn send(&mut self, job: Job) -> u64 {
        self.serial += 1;
        self.post(Work::Make { serial: self.serial, job });
        self.serial
    }

    /// Every interval of seek `epoch` through `hour` has been sent.
    pub(super) fn through(&self, hour: Hour, epoch: u64) {
        self.post(Work::Through { hour, epoch });
    }

    /// Every interval made since the last call for the sends since the last reset, by serial.
    pub(super) fn take(&mut self) -> Vec<(u64, Presented)> {
        let generation = self.generation.load(Ordering::Acquire);
        let mut made = std::mem::take(&mut self.held);
        made.extend(self.made.try_iter().filter(|&(at, ..)| at == generation).map(|(_, serial, presented)| (serial, presented)));
        made
    }

    /// Waits up to `limit` for one more interval while `look`, called at each look, holds; false
    /// when none came in the limit.
    pub(super) fn wait(&mut self, limit: Duration, mut look: impl FnMut() -> bool) -> bool {
        let generation = self.generation.load(Ordering::Acquire);
        let until = Instant::now() + limit;
        loop {
            if !look() {
                return true;
            }
            match self.made.recv_timeout(LOOK.min(until.saturating_duration_since(Instant::now()))) {
                Ok((at, serial, presented)) if at == generation => {
                    self.held.push((serial, presented));
                    return true;
                }
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) if Instant::now() < until => {}
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return false,
            }
        }
    }

    /// Earlier sends are stale: not made if not begun, and not taken.
    pub(super) fn reset(&mut self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.held.clear();
    }

    #[cfg(test)]
    pub(super) fn alive(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.alive)
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.jobs = None;
    }
}

struct Run {
    source: Arc<dyn Source>,
    branch: BranchId,
    stepped: Result<Stepped, String>,
    generation: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    vision: Keyed<Seen, VisionKey>,
}

/// The newest hour handed on: its generation, seek and hour.
type Through = Option<(u64, u64, Hour)>;

impl Run {
    fn run(mut self, received: &Receiver<Sent>, made: &Sender<Made>) {
        let mut pending: VecDeque<(u64, u64, Job)> = VecDeque::new();
        let mut through: Through = None;
        loop {
            let next = if pending.is_empty() {
                received.recv().map(Some).map_err(|_| ())
            } else {
                match received.recv_timeout(LOOK) {
                    Ok(sent) => Ok(Some(sent)),
                    Err(RecvTimeoutError::Timeout) => Ok(None),
                    Err(RecvTimeoutError::Disconnected) => Err(()),
                }
            };
            let Ok(next) = next else { return };
            if self.stop.load(Ordering::Acquire) {
                return;
            }
            match next {
                Some(Sent { generation, work: Work::Make { serial, job } }) => pending.push_back((generation, serial, job)),
                Some(Sent { generation, work: Work::Through { hour, epoch } }) => through = Some((generation, epoch, hour)),
                None => {}
            }
            if !self.progress(&mut pending, through, made) {
                return;
            }
        }
    }

    /// Makes the pending intervals in order while their context is there, then steps the context
    /// to the hour handed on; false when the layer is gone.
    fn progress(&mut self, pending: &mut VecDeque<(u64, u64, Job)>, through: Through, made: &Sender<Made>) -> bool {
        let current = self.generation.load(Ordering::Acquire);
        pending.retain(|&(generation, ..)| generation == current);
        let through = through.filter(|&(generation, ..)| generation == current).map(|(_, epoch, hour)| (epoch, hour));
        while let Some((_, _, job)) = pending.front() {
            if self.stepped.as_ref().is_ok_and(|stepped| job.epoch < stepped.epoch()) {
                pending.pop_front();
                continue;
            }
            let later = through.filter(|&(epoch, _)| epoch == job.epoch).map(|(_, hour)| hour);
            let Some(tables) = self.context(job.at.1, job.epoch, later) else { break };
            let (generation, serial, job) = pending.pop_front().expect("a front");
            if made.send((generation, serial, self.make(job, tables))).is_err() {
                return false;
            }
        }
        if pending.is_empty()
            && let (Ok(stepped), Some((epoch, hour))) = (&mut self.stepped, through)
        {
            stepped.take_through(hour, epoch);
        }
        true
    }

    /// The events' context tables as seek `epoch`'s step through `hour` left them; none until that
    /// step is applied and while a table has not arrived, which a later step through `through` may
    /// bring (the interval is made then).
    fn context(&mut self, hour: Hour, epoch: u64, through: Option<Hour>) -> Option<[SharedView; TABLE_COUNT]> {
        let refused = |reason: &str, tables: &[TableId]| -> [SharedView; TABLE_COUNT] {
            std::array::from_fn(|index| {
                let table = TableId::ALL[index];
                if tables.contains(&table) { SharedView::from(TableView::Unavailable { reason: reason.to_owned() }) } else { not_declared(table) }
            })
        };
        let stepped = match &mut self.stepped {
            Ok(stepped) => stepped,
            Err(why) => return Some(refused(why, &worker_tables())),
        };
        let reached = stepped.take_through(hour, epoch).is_some_and(|last| last >= hour);
        let frame = match stepped.frame(self.branch, hour).filter(|_| reached) {
            Some(frame) => frame,
            None => {
                let later = through.filter(|&through| through > hour)?;
                stepped.take_through(later, epoch).filter(|&last| last >= hour)?;
                stepped.frame(self.branch, hour)?
            }
        };
        let mut tables = frame.tables;
        let absent = stepped.absent();
        if !absent.is_empty() {
            let why = refusal("the map text's events", absent);
            for &table in absent {
                tables[table.index()] = SharedView::from(TableView::Unavailable { reason: why.clone() });
            }
        }
        Some(tables)
    }

    fn make(&self, job: Job, tables: [SharedView; TABLE_COUNT]) -> Presented {
        let Job { at, seek, interval, choice, .. } = job;
        let mut frame = DecodedFrame { branch: self.branch, hour: at.1, tables, metrics: Metrics::default() };
        let source = &*self.source;
        let seen = match cached_vision(&frame, &choice, FogContent::loaded(source.fog()), &self.vision) {
            Ok(vision) => vision,
            Err(error) => return Presented::Missing { at, seek, reason: format!("map text presented hour failed: {error}") },
        };
        frame.tables[TableId::MapEvents.index()] = interval;
        Presented::Hour { at, seek, prepared: Arc::new(Prepared::read(&frame, &seen, source.text(), source.localisation(), |province| source.place(province))) }
    }
}
