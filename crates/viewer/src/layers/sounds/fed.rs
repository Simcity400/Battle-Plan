//! The window's battle ambience: the battle list ([`Sounds`]) made on a worker the layer owns, fed
//! the steps the frame applied and each declaration. The worker keeps its own copies of the
//! tables it reads (made at a compose, kept by applying each step as the frame's mirrors are),
//! settles every mark whole and hands back the mix's battles stamped with the hour applied. The
//! frame never waits for them while playing: it swaps in what has arrived and shows the hour of the
//! battles it has.

use super::{Columns, Sounds, TABLES};
use crate::{
    layers::{Budget, Carried, Fed as Products, Gpu, Inputs, Layer, Published, Scope, Scoped, Sight, View},
    playhead_host::mirror::{Mirror, Mirrors},
};
use playhead::Step;
use replay_data::Hour;
use std::{
    collections::VecDeque,
    io,
    sync::{Arc, mpsc},
    thread,
};
use viewer_audio::battle::{self, Battles};

enum Job {
    /// The battle list made anew from the tables' copies.
    Reset { epoch: u64, model: Box<Sounds>, mirrors: Box<Mirrors> },
    /// One frame's steps in order and the columns when the view declared again; the battles
    /// stamped `stamp`.
    Frame { epoch: u64, stamp: Option<Hour>, steps: Vec<Arc<Step>>, declared: Option<Option<Columns>> },
}

/// What the worker made for `answers` jobs, current at `stamp` (none: the compose's).
struct Made {
    epoch: u64,
    stamp: Option<Hour>,
    answers: usize,
    battles: Option<Arc<Battles>>,
    marked: [usize; 3],
    made: u64,
    scoped: Scoped,
    held: usize,
}

/// The worker's battle list and the copies of the tables it reads.
struct Work {
    model: Box<Sounds>,
    mirrors: Box<Mirrors>,
}

impl Work {
    /// A frame's steps applied to the copies and marked; a declaration taken as the frame's layer
    /// takes it.
    fn take(&mut self, steps: Vec<Arc<Step>>, declared: Option<Option<Columns>>) {
        if let Some(columns) = declared {
            (self.model.columns, self.model.whole) = (columns, true);
        }
        for step in steps {
            self.mirrors.apply(&step);
            self.model.apply_step(&step);
        }
    }

    fn made(&mut self, epoch: u64, stamp: Option<Hour>, answers: usize) -> Made {
        let held = self.model.held() + self.mirrors.iter().map(Mirror::bytes).sum::<usize>();
        let battles = self.model.published.lock().unwrap_or_else(|p| p.into_inner()).clone();
        Made { epoch, stamp, answers, battles, marked: self.model.marked, made: self.model.made, scoped: std::mem::take(&mut self.model.scoped), held }
    }
}

fn run(jobs: mpsc::Receiver<Job>, made: mpsc::Sender<Made>) {
    let mut work: Option<Work> = None;
    let mut epoch = 0;
    let mut next = None;
    loop {
        let job = match next.take() {
            Some(job) => job,
            None => match jobs.recv() {
                Ok(job) => job,
                Err(_) => return,
            },
        };
        let product = match job {
            Job::Reset { epoch: of, mut model, mirrors } => {
                epoch = of;
                let _ = model.compose(&mirrors);
                work.insert(Work { model, mirrors }).made(epoch, None, 1)
            }
            // Jobs go in the order sent, each epoch's after its reset: one of an epoch composed again
            // is dropped by the frame.
            Job::Frame { epoch: of, .. } if of != epoch => continue,
            Job::Frame { stamp, steps, declared, .. } => {
                let Some(held) = &mut work else { continue };
                held.take(steps, declared);
                // The jobs queued meanwhile go into the same battles, made for the newest hour.
                let (mut stamp, mut answers) = (stamp, 1);
                while let Ok(queued) = jobs.try_recv() {
                    match queued {
                        Job::Frame { epoch: of, stamp: at, steps, declared } if of == epoch => {
                            held.take(steps, declared);
                            (stamp, answers) = (at, answers + 1);
                        }
                        Job::Frame { .. } => {}
                        reset @ Job::Reset { .. } => {
                            next = Some(reset);
                            break;
                        }
                    }
                }
                held.model.settle_dirty(&held.mirrors, &mut Budget::unlimited());
                held.made(epoch, stamp, answers)
            }
        };
        if made.send(product).is_err() {
            return;
        }
    }
}

/// The window's battle ambience layer: what its view declares, the frame's steps handed to its
/// worker, and the worker's newest battles due.
pub(crate) struct SoundsFed {
    /// The battle list as the view declares it: never composed or settled here.
    spec: Sounds,
    jobs: Option<mpsc::Sender<Job>>,
    made: Option<mpsc::Receiver<Made>>,
    epoch: u64,
    /// Jobs of this epoch sent whose battles have not been received.
    owed: usize,
    products: Products<Made>,
    /// The steps handed and not yet sent with a frame's job, in order.
    waiting: VecDeque<Arc<Step>>,
    /// A declaration made since the last job.
    declared: bool,
    /// The newest battles taken: the mix the audio plays.
    battles: Option<Arc<Battles>>,
    marked: [usize; 3],
    made_battles: u64,
    scoped: Scoped,
    worker_held: usize,
    /// The worker stopped (it panicked): no battle sounds from then on.
    stopped: bool,
}

impl SoundsFed {
    /// The window's layer over `sounds`.
    pub(crate) fn new(sounds: Sounds) -> Self {
        Self {
            spec: sounds,
            jobs: None,
            made: None,
            epoch: 0,
            owed: 0,
            products: Products::default(),
            waiting: VecDeque::new(),
            declared: false,
            battles: None,
            marked: [0; 3],
            made_battles: 0,
            scoped: Scoped::default(),
            worker_held: 0,
            stopped: false,
        }
    }

    /// The battles the audio plays, as last settled whole; none when the content plays none.
    pub(crate) fn battles(&self) -> Option<&Arc<Battles>> {
        self.battles.as_ref()
    }

    /// Combats, provinces and battles the worker's steps marked, as of the newest battles.
    pub(crate) fn marked(&self) -> [usize; 3] {
        self.marked
    }

    /// Battles its worker made (their members' fronts summed and named), as of the newest battles.
    #[cfg(test)]
    pub(crate) fn made(&self) -> u64 {
        self.made_battles
    }

    /// A battle list as the view declares it, made from nothing, as the worker's is at a compose.
    pub(crate) fn fresh(&self) -> Sounds {
        self.spec.fresh()
    }

    fn send(&mut self, job: Job) {
        if let Some(jobs) = &self.jobs
            && jobs.send(job).is_ok()
        {
            self.owed += 1;
        }
    }

    /// Receives what the worker made and takes every product due, in order. With `wait` (a whole
    /// settle, a compose), until every job sent is answered; otherwise what has arrived.
    fn exchange(&mut self, wait: bool) {
        if let Some(receive) = &self.made {
            loop {
                let made = if wait && self.owed > 0 { receive.recv().map_err(|_| mpsc::TryRecvError::Disconnected) } else { receive.try_recv() };
                let made = match made {
                    Ok(made) => made,
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        (self.owed, self.stopped) = (0, true);
                        break;
                    }
                };
                if made.epoch == self.epoch {
                    self.owed -= made.answers;
                    self.products.receive(made.stamp, made);
                }
            }
        }
        let mut due = Vec::new();
        self.products.take(|made| due.push(made));
        for made in due {
            (self.battles, self.marked, self.made_battles, self.worker_held) = (made.battles, made.marked, made.made, made.held);
            self.scoped.extend(made.scoped);
        }
        if self.stopped {
            self.battles = None;
        }
    }
}

impl Layer for SoundsFed {
    fn name(&self) -> &'static str {
        "sounds"
    }

    fn placed(&mut self, view: &View) -> Option<u64> {
        self.spec.placed(view)
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        self.declared = true;
        self.spec.inputs(view, inputs)
    }

    /// The worker started from the tables' copies (made here, off the frame thread); its first
    /// battles waited for.
    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        if self.jobs.is_none() {
            let (jobs, receive) = mpsc::channel();
            let (send, made) = mpsc::channel();
            thread::Builder::new().name("battle sounds".into()).spawn(move || {
                viewer_foundation::worker_priority::background_priority();
                run(receive, send)
            })?;
            (self.jobs, self.made) = (Some(jobs), Some(made));
        }
        let mut copies = Box::<Mirrors>::default();
        for table in TABLES {
            if let Some(mirror) = mirrors.get(table) {
                copies.insert(Box::new(mirror.clone()));
            }
        }
        let mut model = Box::new(self.fresh());
        (model.mix_inline, model.marked) = (true, self.marked);
        self.declared = false;
        self.epoch += 1;
        self.owed = 0;
        self.products.compose(mirrors.iter().find_map(Mirror::head));
        self.waiting.clear();
        self.send(Job::Reset { epoch: self.epoch, model, mirrors: copies });
        self.exchange(true);
        Ok(())
    }

    fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
        self.products.applied(step);
    }

    fn wants_ahead(&self) -> bool {
        true
    }

    /// The steps the frame applied (and those built past it), kept until the frame's job goes.
    fn ahead(&mut self, steps: &[Arc<Step>]) {
        self.waiting.extend(steps.iter().cloned());
    }

    /// Its worker is given only steps applied, so it folded nothing void: the steps kept past `kept`
    /// go, and are handed again.
    fn void_ahead(&mut self, _mirrors: &Mirrors, kept: Option<Hour>) {
        self.waiting.retain(|step| kept.is_some_and(|kept| step.last() <= kept));
    }

    /// What the worker made taken, never waited for while playing; this frame's steps and a new
    /// declaration then go to the worker in one job. A whole settle waits for the battles of its
    /// hour.
    fn settle(&mut self, _mirrors: &Mirrors, budget: &mut Budget) -> io::Result<()> {
        let whole = budget.unlimited_now();
        self.exchange(false);
        let applied = self.products.applied_hour();
        let mut steps = Vec::new();
        while self.waiting.front().is_some_and(|step| applied.is_some_and(|applied| step.last() <= applied)) {
            steps.extend(self.waiting.pop_front());
        }
        let declared = std::mem::take(&mut self.declared).then_some(self.spec.columns);
        if !steps.is_empty() || declared.is_some() {
            self.send(Job::Frame { epoch: self.epoch, stamp: applied, steps, declared });
        }
        self.exchange(whole);
        Ok(())
    }

    fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }

    fn carried(&self) -> Carried {
        // Every mark is the worker's: published, not carried.
        Carried::default()
    }

    /// Current once the battles of the hour applied are taken; while a step is with the worker, the
    /// hour of the battles it has.
    fn published(&self) -> Published {
        if self.stopped {
            return Published::Nothing;
        }
        self.products.published()
    }

    fn scope(&mut self, sight: &Sight<'_>) -> Option<Scope> {
        let hearing = self.spec.audio.as_deref().and_then(battle::hearing);
        Some(self.scoped.take(sight, hearing))
    }

    fn held(&self) -> usize {
        self.worker_held + self.waiting.capacity() * 8 + self.scoped.bytes()
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}
