//! The window's base counters: the counters ([`BaseCounters`]) made on a worker the layer owns,
//! fed the steps the frame applied. The worker keeps its own copies of the tables they read (made at
//! a compose, kept by applying each step as the frame's mirrors are), settles every mark whole and
//! hands back the values stamped with the hour applied. The frame never waits for them while
//! playing: it swaps in what has arrived and shows the hour of the values it has.

use super::{BaseCounters, TABLES};
use crate::{
    layers::{Budget, Carried, Fed as Products, Gpu, Inputs, Layer, Published, View},
    playhead_host::mirror::{Mirror, Mirrors},
};
use playhead::Step;
use replay_data::{Hour, rows::TableId};
use std::{
    collections::VecDeque,
    io,
    sync::{Arc, atomic::Ordering, mpsc},
    thread,
};
use viewer_counters::map::counters::BaseValues;
use viewer_foundation::map::perspective::{Choice, Vision};

enum Job {
    /// The counters made anew from the tables' copies.
    Reset { epoch: u64, model: Box<BaseCounters>, mirrors: Box<Mirrors> },
    /// One frame's steps in order and a vision given whole; the values stamped `stamp`.
    Frame { epoch: u64, stamp: Option<Hour>, steps: Vec<Arc<Step>>, given: Option<(Choice, Arc<Vision>)> },
}

/// What the worker made for `answers` jobs, current at `stamp` (none: the compose's).
struct Made {
    epoch: u64,
    stamp: Option<Hour>,
    answers: usize,
    values: Option<Arc<BaseValues>>,
    refused: Option<String>,
    made: usize,
    work: u64,
    held: usize,
}

/// The worker's counters and the copies of the tables they read.
struct Work {
    model: Box<BaseCounters>,
    mirrors: Box<Mirrors>,
}

impl Work {
    /// A frame's steps applied to the copies and marked, and a vision given.
    fn take(&mut self, steps: Vec<Arc<Step>>, given: Option<(Choice, Arc<Vision>)>) {
        for step in steps {
            self.mirrors.apply(&step);
            self.model.apply(&step, &self.mirrors);
        }
        if let Some((choice, vision)) = given {
            self.model.set_vision(choice, vision);
        }
    }

    fn made(&self, epoch: u64, stamp: Option<Hour>, answers: usize) -> Made {
        let held = self.model.held() + self.mirrors.iter().map(Mirror::bytes).sum::<usize>();
        Made { epoch, stamp, answers, values: self.model.values.clone(), refused: self.model.refused.clone(), made: self.model.made, work: self.model.work, held }
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
            Job::Frame { stamp, steps, given, .. } => {
                let Some(held) = &mut work else { continue };
                held.take(steps, given);
                // The jobs queued meanwhile go into the same values, made for the newest hour.
                let (mut stamp, mut answers) = (stamp, 1);
                while let Ok(queued) = jobs.try_recv() {
                    match queued {
                        Job::Frame { epoch: of, stamp: at, steps, given } if of == epoch => {
                            held.take(steps, given);
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

/// The window's base counters layer: what its view declares, the frame's steps handed to its
/// worker, and the worker's newest values due.
pub(crate) struct BaseCountersFed {
    /// The counters as the view declares them, and the vision given them: never composed or settled
    /// here.
    spec: BaseCounters,
    jobs: Option<mpsc::Sender<Job>>,
    made: Option<mpsc::Receiver<Made>>,
    epoch: u64,
    /// Jobs of this epoch sent whose values have not been received.
    owed: usize,
    products: Products<Made>,
    /// The steps handed and not yet sent with a frame's job, in order.
    waiting: VecDeque<Arc<Step>>,
    /// The newest values taken.
    values: Option<Arc<BaseValues>>,
    refused: Option<String>,
    units: usize,
    work: u64,
    worker_held: usize,
    /// The worker stopped (it panicked): no base icons are drawn from then on.
    stopped: bool,
}

impl BaseCountersFed {
    /// The window's layer over `counters`.
    pub(crate) fn new(counters: BaseCounters) -> Self {
        Self {
            spec: counters,
            jobs: None,
            made: None,
            epoch: 0,
            owed: 0,
            products: Products::default(),
            waiting: VecDeque::new(),
            values: None,
            refused: Some("not composed yet".into()),
            units: 0,
            work: 0,
            worker_held: 0,
            stopped: false,
        }
    }

    pub(crate) fn set_vision(&self, choice: Choice, vision: Arc<Vision>) {
        self.spec.set_vision(choice, vision);
    }

    pub(crate) fn perspective(&self) -> Option<(Choice, Arc<Vision>)> {
        self.spec.perspective()
    }

    pub(crate) fn refused(&self) -> Option<&str> {
        if self.stopped {
            return Some("the base counters' worker stopped");
        }
        self.refused.as_deref()
    }

    /// The view does not draw the base icons.
    pub(crate) fn hidden(&self) -> bool {
        self.spec.hidden()
    }

    pub(crate) fn values(&self) -> Option<&BaseValues> {
        self.values.as_deref()
    }

    /// The values the window's base icons draw; none while refused or hidden.
    pub(crate) fn published(&self) -> Option<&Arc<BaseValues>> {
        self.values.as_ref().filter(|_| self.refused().is_none() && !self.hidden())
    }

    /// Units its worker made and rows it read or changed, as of the newest values.
    pub(crate) fn made(&self) -> usize {
        self.units
    }

    pub(crate) fn work(&self) -> u64 {
        self.work
    }

    /// A model as the view declares it, made from nothing, as the worker's is at a compose.
    pub(crate) fn fresh(&self) -> BaseCounters {
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
            (self.values, self.refused, self.units, self.work, self.worker_held) = (made.values, made.refused, made.made, made.work, made.held);
        }
        if self.stopped {
            self.values = None;
        }
    }
}

impl Layer for BaseCountersFed {
    fn name(&self) -> &'static str {
        "base_counters"
    }

    fn placed(&mut self, view: &crate::layers::View) -> Option<u64> {
        self.spec.placed(view)
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        self.spec.inputs(view, inputs)
    }

    /// The worker started from the tables' copies (made here, off the frame thread); its first
    /// values waited for.
    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        if self.jobs.is_none() {
            let (jobs, receive) = mpsc::channel();
            let (send, made) = mpsc::channel();
            thread::Builder::new().name("base counters".into()).spawn(move || {
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
        self.spec.vision_new.store(false, Ordering::Release);
        let model = Box::new(self.fresh());
        self.epoch += 1;
        self.owed = 0;
        self.products.compose(mirrors.get(TableId::Provinces).and_then(Mirror::head));
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

    /// What the worker made taken, never waited for while playing; this frame's steps and a vision
    /// given then go to the worker in one job. A whole settle waits for the values of its hour.
    fn settle(&mut self, _mirrors: &Mirrors, budget: &mut Budget) -> io::Result<()> {
        let whole = budget.unlimited_now();
        self.exchange(false);
        let applied = self.products.applied_hour();
        let mut steps = Vec::new();
        while self.waiting.front().is_some_and(|step| applied.is_some_and(|applied| step.last() <= applied)) {
            steps.extend(self.waiting.pop_front());
        }
        let given = self.spec.vision_new.swap(false, Ordering::AcqRel).then(|| self.spec.perspective()).flatten();
        if !steps.is_empty() || given.is_some() {
            self.send(Job::Frame { epoch: self.epoch, stamp: applied, steps, given });
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

    /// Current once the values of the hour applied are taken; while a step is with the worker, the
    /// hour of the values it has.
    fn published(&self) -> Published {
        if self.stopped {
            return Published::Nothing;
        }
        self.products.published()
    }

    fn held(&self) -> usize {
        self.worker_held + self.waiting.capacity() * 8
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}
