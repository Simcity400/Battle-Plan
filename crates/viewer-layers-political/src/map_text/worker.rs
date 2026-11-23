
use super::{Presented, Source, worker_tables};
use viewer_layer_core::{rebuilt::refusal, playhead_host::views::Stepped};
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
use viewer_runtime::memo::Keyed;
use viewer_foundation::{
    perspective::{Choice, VISION_TABLES, VisionKey, Vision as Seen, cached_vision},
    visibility::FogContent,
};
use viewer_map_text::animated_map_text::events::Prepared;

const LOOK: Duration = Duration::from_millis(50);

pub(super) struct Job {
    pub at: (BranchId, Hour),
    pub seek: bool,
    pub epoch: u64,
    pub interval: SharedView,
    pub choice: Choice,
}

enum Work {
    Make { serial: u64, job: Job },
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
    #[cfg(any(test, feature = "test-support"))]
    alive: Arc<AtomicBool>,
    held: Vec<(u64, Presented)>,
}

impl Worker {
    pub(super) fn start(source: Arc<dyn Source>, branch: BranchId, stepped: Result<Stepped, String>) -> io::Result<Self> {
        let (jobs, received) = mpsc::channel::<Sent>();
        let (made, taken) = mpsc::channel::<Made>();
        let generation = Arc::new(AtomicU64::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let alive = Arc::new(AtomicBool::new(true));
        let run = Run { source, branch, stepped, generation: Arc::clone(&generation), stop: Arc::clone(&stop), vision: Keyed::new(VISION_TABLES) };
        let living = Arc::clone(&alive);
        std::thread::Builder::new().name("map text".into()).spawn(move || {
            viewer_runtime::worker_priority::enter(viewer_runtime::worker_priority::Class::Kept);
            run.run(&received, &made);
            living.store(false, Ordering::Release);
        })?;
        Ok(Self {
            jobs: Some(jobs),
            made: taken,
            generation,
            serial: 0,
            stop,
            #[cfg(any(test, feature = "test-support"))]
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

    pub(super) fn send(&mut self, job: Job) -> u64 {
        self.serial += 1;
        self.post(Work::Make { serial: self.serial, job });
        self.serial
    }

    pub(super) fn through(&self, hour: Hour, epoch: u64) {
        self.post(Work::Through { hour, epoch });
    }

    pub(super) fn take(&mut self) -> Vec<(u64, Presented)> {
        let generation = self.generation.load(Ordering::Acquire);
        let mut made = std::mem::take(&mut self.held);
        made.extend(self.made.try_iter().filter(|&(at, ..)| at == generation).map(|(_, serial, presented)| (serial, presented)));
        made
    }

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

    pub(super) fn reset(&mut self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.held.clear();
    }

    #[cfg(any(test, feature = "test-support"))]
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
                stepped.frame_past(self.branch, hour)?
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
