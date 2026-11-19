//! The window's counters layer: the counters ([`Counters`]) made on a worker the layer owns, fed
//! when the frame applies a step. The worker keeps its own copies of the mirrors the counters read
//! (made at a compose, kept by applying each step's change sets as the frame's are) and a copy of
//! what a division's sight reads of the vision layer, kept from the layer's changes as the frame
//! has them ([`SightCopy`]); the relations layer's index and wars are shared. Each frame that applied
//! steps, or whose vision, relations, view or given vision changed, hands the worker one job: that
//! frame's steps in order and those changes. The worker applies them, settles every mark whole (the
//! stacks the camera admits) and makes a product stamped with the hour applied: the land stacks, a
//! new allocation when their content changed, each source's members kept where they did not. Jobs
//! queued while it worked are made into one product, for the newest. The frame never waits for it
//! while playing: it takes what has arrived, and shows the hour of the product it has.

use super::{Admits, Counters, PHASES, Sees, Source, TABLES, action};
use crate::{
    layers::{
        Carried, Settling, Fed as Products, Inputs, Layer, Published, Upstream, View,
        relations::{self, Relations},
        vision::{SightChange, SightCopy, SightKeys, Vision as VisionLayer},
    },
    playhead_host::mirror::{Mirror, Mirrors},
};
use playhead::Step;
use replay_data::{Hour, rows::TableId};
use std::{
    collections::VecDeque,
    io,
    sync::{Arc, atomic::Ordering, mpsc},
    thread,
    time::Instant,
};
use viewer_counters::map::counters::{LandStacks, Member, type_tint};
use viewer_foundation::map::{
    camera::Placement,
    counter_action::Action,
    perspective::{Choice, Vision},
};

/// The relations layer's index and wars, when they changed.
type Relating = Option<(Arc<type_tint::Index>, Option<Arc<relations::Facts>>)>;

/// The view as the counters read it: the camera and viewport, and whether land counters are drawn.
#[derive(Clone, Copy, PartialEq)]
struct Viewing {
    camera: Option<(Placement, [u32; 2])>,
    hidden: bool,
}

/// One frame's changes: its steps in order, what changed of the vision's sight, the relations and
/// the view, and a vision given whole (the proof's); the product stamped `stamp`.
struct Changes {
    stamp: Option<Hour>,
    steps: Vec<Arc<Step>>,
    sight: Option<SightChange>,
    relating: Relating,
    viewing: Option<Viewing>,
    given: Option<(Choice, Arc<Vision>)>,
}

enum Job {
    /// The counters made anew from the mirrors' copies, with the vision's sight whole.
    Reset { epoch: u64, model: Box<Counters>, mirrors: Box<Mirrors>, sight: Option<Box<SightCopy>>, relating: Relating },
    Frame { epoch: u64, changes: Changes },
}

/// What the worker made for `answers` jobs, current at `stamp` (none: the compose's).
struct Made {
    epoch: u64,
    stamp: Option<Hour>,
    answers: usize,
    stacks: Arc<LandStacks>,
    admits: Option<Admits>,
    refused: Option<String>,
    marked: usize,
    phases: [(u64, u64); PHASES.len()],
    held: usize,
    made_ns: u64,
}

/// The worker's counters, their mirrors and their sight.
struct Work {
    model: Box<Counters>,
    mirrors: Box<Mirrors>,
    sight: Option<SightCopy>,
}

impl Work {
    fn relate(&mut self, relating: Relating) {
        if let Some((index, wars)) = relating {
            self.model.relating(index, wars);
        }
    }

    /// One frame's changes taken: its steps applied to the mirrors and marked, then the relations,
    /// the vision given, the view and the sight.
    fn take(&mut self, changes: Changes) {
        let Changes { steps, sight, relating, viewing, given, .. } = changes;
        for step in steps {
            self.mirrors.apply(&step);
            self.model.apply(&step, &self.mirrors);
        }
        self.relate(relating);
        if let Some((choice, vision)) = given {
            self.model.set_vision(choice, vision);
        }
        if let Some(viewing) = viewing {
            self.model.all |= self.model.hidden && !viewing.hidden;
            self.model.hidden = viewing.hidden;
            if viewing.camera != self.model.camera {
                (self.model.camera, self.model.camera_dirty) = (viewing.camera, true);
            }
        }
        if let Some(change) = &sight {
            match &mut self.sight {
                Some(copy) => copy.apply(change),
                None => self.sight = change.whole().cloned(),
            }
            if let Some(copy) = &self.sight {
                self.model.take_sight((copy.choice().clone(), copy.sight_epoch()), change.whole().is_some(), change.provinces(), change.units(), &self.mirrors);
            }
        }
    }

    /// The marks settled whole and the touched stacks published.
    fn make(&mut self, epoch: u64, stamp: Option<Hour>, answers: usize, started: Instant) -> Made {
        let content = Arc::clone(&self.model.content);
        let (model, mirrors, sight) = (&mut self.model, &*self.mirrors, self.sight.as_ref());
        content.scoped(|| model.settle_dirty(mirrors, sight.map(Sees::Copy)));
        self.model.publish();
        let held = self.model.held() + self.mirrors.iter().map(Mirror::bytes).sum::<usize>() + self.sight.as_ref().map_or(0, SightCopy::bytes);
        Made {
            epoch,
            stamp,
            answers,
            stacks: Arc::clone(&self.model.published),
            admits: self.model.admits.clone(),
            refused: self.model.refused.clone(),
            marked: self.model.marked,
            phases: self.model.phases,
            held,
            made_ns: started.elapsed().as_nanos() as u64,
        }
    }
}

/// The tests' hold on the worker: it waits at each job it receives while held, and counts the jobs
/// it has answered.
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct Gate {
    held: std::sync::Mutex<bool>,
    open: std::sync::Condvar,
    answered: std::sync::atomic::AtomicUsize,
}

#[cfg(any(test, feature = "test-support"))]
impl Gate {
    fn pass(&self) {
        let mut held = self.held.lock().unwrap();
        while *held {
            held = self.open.wait(held).unwrap();
        }
    }

    fn answer(&self, jobs: usize) {
        self.answered.fetch_add(jobs, Ordering::AcqRel);
    }
}

fn run(jobs: mpsc::Receiver<Job>, made: mpsc::Sender<Made>, #[cfg(any(test, feature = "test-support"))] gate: Arc<Gate>) {
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
        #[cfg(any(test, feature = "test-support"))]
        gate.pass();
        let started = Instant::now();
        let product = match job {
            Job::Reset { epoch: of, model, mirrors, sight, relating } => {
                epoch = of;
                let held = work.insert(Work { model, mirrors, sight: sight.map(|sight| *sight) });
                held.relate(relating);
                let content = Arc::clone(&held.model.content);
                let (model, mirrors, sight) = (&mut held.model, &*held.mirrors, held.sight.as_ref());
                let _ = content.scoped(|| model.compose_reading(mirrors, sight.map(Sees::Copy)));
                held.make(epoch, None, 1, started)
            }
            // Jobs go in the order sent, each epoch's after its reset: one of an epoch restarted is
            // dropped by the frame.
            Job::Frame { epoch: of, .. } if of != epoch => {
                #[cfg(any(test, feature = "test-support"))]
                gate.answer(1);
                continue;
            }
            Job::Frame { changes, .. } => {
                let Some(held) = &mut work else { continue };
                let mut stamp = changes.stamp;
                held.take(changes);
                // The jobs queued meanwhile go into the same product: their marks are settled
                // together, each stack from the mirrors at the newest hour, as a compose there would.
                let mut answers = 1;
                while let Ok(queued) = jobs.try_recv() {
                    match queued {
                        Job::Frame { epoch: of, changes } if of == epoch => {
                            stamp = changes.stamp;
                            held.take(changes);
                            answers += 1;
                        }
                        Job::Frame { .. } => {
                            #[cfg(any(test, feature = "test-support"))]
                            gate.answer(1);
                        }
                        reset @ Job::Reset { .. } => {
                            next = Some(reset);
                            break;
                        }
                    }
                }
                held.make(epoch, stamp, answers, started)
            }
        };
        #[cfg(any(test, feature = "test-support"))]
        let answers = product.answers;
        if made.send(product).is_err() {
            return;
        }
        #[cfg(any(test, feature = "test-support"))]
        gate.answer(answers);
    }
}

/// What the frame saw of its worker while playing, for the paced report.
#[derive(Clone, Copy, Debug, Default)]
pub struct Tally {
    /// Products taken, the jobs they answered, and the worker's time making them.
    pub products: u64,
    pub jobs: u64,
    pub made_ns: u64,
    pub made_ns_worst: u64,
}

/// The window's counters layer: what its view declares, the frame's steps and changes handed to its
/// worker, and the worker's newest product due.
pub struct Fed {
    /// The counters as the view declares them, and the vision given them: never composed or settled
    /// here.
    spec: Counters,
    jobs: Option<mpsc::Sender<Job>>,
    made: Option<mpsc::Receiver<Made>>,
    epoch: u64,
    /// Jobs of this epoch sent whose product has not been received.
    owed: usize,
    products: Products<Made>,
    /// The steps handed and not yet sent with a frame's changes, in order.
    waiting: VecDeque<Arc<Step>>,
    /// What was last handed the worker of the vision's sight, the relations and the view.
    sight: Option<SightKeys>,
    relating: Relating,
    viewing: Option<Viewing>,
    /// The newest product taken.
    stacks: Option<Arc<LandStacks>>,
    admits: Option<Admits>,
    refused: Option<String>,
    marked: usize,
    phases: [(u64, u64); PHASES.len()],
    worker_held: usize,
    /// The worker stopped (it panicked): no stacks are drawn from then on.
    stopped: bool,
    tally: Tally,
    #[cfg(any(test, feature = "test-support"))]
    gate: Arc<Gate>,
    /// Jobs sent, the waits for the worker, and the products taken, since the layer was made.
    #[cfg(any(test, feature = "test-support"))]
    sent: usize,
    #[cfg(any(test, feature = "test-support"))]
    blocked: u64,
    #[cfg(any(test, feature = "test-support"))]
    taken: u64,
}

impl Fed {
    /// The window's layer over `counters`.
    pub fn new(counters: Counters) -> Self {
        Self {
            spec: counters,
            jobs: None,
            made: None,
            epoch: 0,
            owed: 0,
            products: Products::default(),
            waiting: VecDeque::new(),
            sight: None,
            relating: None,
            viewing: None,
            stacks: None,
            admits: None,
            refused: Some("not composed yet".into()),
            marked: 0,
            phases: [(0, 0); PHASES.len()],
            worker_held: 0,
            stopped: false,
            tally: Tally::default(),
            #[cfg(any(test, feature = "test-support"))]
            gate: Arc::default(),
            #[cfg(any(test, feature = "test-support"))]
            sent: 0,
            #[cfg(any(test, feature = "test-support"))]
            blocked: 0,
            #[cfg(any(test, feature = "test-support"))]
            taken: 0,
        }
    }

    /// The land stacks of the newest product taken.
    pub fn stacks(&self) -> Option<&Arc<LandStacks>> {
        self.stacks.as_ref()
    }

    /// Each source of the newest product with its members and their actions.
    pub fn sources_acting(&self) -> impl Iterator<Item = (Source, impl Iterator<Item = (&Member, Action)> + '_)> + '_ {
        self.stacks.iter().flat_map(|stacks| stacks.iter()).map(|(&source, members)| (source, members.iter().map(|(member, action)| (member, *action))))
    }

    /// Whether the newest product's camera admits `province`.
    pub fn admitted(&self, province: u32) -> bool {
        self.admits.as_ref().is_none_or(|admits| admits(province))
    }

    /// Why the newest product draws nothing, by name.
    pub fn refused(&self) -> Option<&str> {
        if self.stopped {
            return Some("the counters' worker stopped");
        }
        self.refused.as_deref()
    }

    /// The view draws no land counter.
    pub fn hidden(&self) -> bool {
        self.spec.hidden()
    }

    /// The vision given whole (the proof's): handed to the worker with the next frame's changes.
    pub fn set_vision(&self, choice: Choice, vision: Arc<Vision>) -> bool {
        self.spec.set_vision(choice, vision)
    }

    pub fn perspective(&self) -> Option<(Choice, Arc<Vision>)> {
        self.spec.perspective()
    }

    /// Divisions the worker's steps have marked since its compose, and its phases, as of the newest
    /// product.
    pub fn marked(&self) -> usize {
        self.marked
    }

    pub fn phases(&self) -> impl Iterator<Item = (&'static str, u64, u64)> + '_ {
        PHASES.iter().zip(&self.phases).map(|(&name, &(entities, ns))| (name, entities, ns))
    }

    pub fn tally(&self) -> Tally {
        self.tally
    }

    /// A model as the view declares it, made from nothing, as the worker's is at a compose.
    pub fn fresh(&self) -> Counters {
        let mut model = self.spec.fresh();
        model.camera_dirty = model.camera.is_some();
        model
    }

    /// Holds the worker at the next job it receives, or lets it go on.
    #[cfg(any(test, feature = "test-support"))]
    pub fn hold_worker(&self, held: bool) {
        *self.gate.held.lock().unwrap() = held;
        self.gate.open.notify_all();
    }

    /// The worker has answered every job sent.
    #[cfg(any(test, feature = "test-support"))]
    pub fn answered(&self) -> bool {
        self.gate.answered.load(Ordering::Acquire) == self.sent
    }

    /// The times the frame waited for the worker, and the products it took.
    #[cfg(any(test, feature = "test-support"))]
    pub fn blocked(&self) -> u64 {
        self.blocked
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn taken(&self) -> u64 {
        self.taken
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn applied_hour(&self) -> Option<Hour> {
        self.products.applied_hour()
    }

    fn viewing(&self) -> Viewing {
        Viewing { camera: self.spec.camera, hidden: self.spec.hidden }
    }

    fn send(&mut self, job: Job) {
        if let Some(jobs) = &self.jobs
            && jobs.send(job).is_ok()
        {
            self.owed += 1;
            #[cfg(any(test, feature = "test-support"))]
            {
                self.sent += 1;
            }
        }
    }

    /// Receives what the worker made and takes every product due, in order. With `wait` (a whole
    /// settle, a compose), until every job sent is answered; otherwise what has arrived.
    fn exchange(&mut self, wait: bool) {
        if let Some(receive) = &self.made {
            loop {
                let made = if wait && self.owed > 0 {
                    #[cfg(any(test, feature = "test-support"))]
                    {
                        self.blocked += 1;
                    }
                    receive.recv().map_err(|_| mpsc::TryRecvError::Disconnected)
                } else {
                    receive.try_recv()
                };
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
            let tally = &mut self.tally;
            (tally.products, tally.jobs, tally.made_ns, tally.made_ns_worst) = (tally.products + 1, tally.jobs + made.answers as u64, tally.made_ns + made.made_ns, tally.made_ns_worst.max(made.made_ns));
            (self.stacks, self.admits, self.refused, self.marked, self.phases, self.worker_held) = (Some(made.stacks), made.admits, made.refused, made.marked, made.phases, made.held);
            #[cfg(any(test, feature = "test-support"))]
            {
                self.taken += 1;
            }
        }
        if self.stopped {
            self.stacks = None;
        }
    }
}

impl Layer for Fed {
    fn name(&self) -> &'static str {
        "counters"
    }

    fn placed(&mut self, view: &crate::layers::View) -> Option<u64> {
        self.spec.placed(view)
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        self.spec.inputs(view, inputs)
    }

    fn moved_at(&self, step: &playhead::Step, mirrors: &Mirrors, into: &mut viewer_foundation::map::counter_move::MovedHours) {
        self.spec.moved_at(step, mirrors, into);
    }

    /// The worker started from the mirrors' copies (made here, off the frame thread), the vision's
    /// sight whole and the relations; its first product waited for.
    fn compose_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>) -> io::Result<()> {
        if self.jobs.is_none() {
            let (jobs, receive) = mpsc::channel();
            let (send, made) = mpsc::channel();
            #[cfg(any(test, feature = "test-support"))]
            let gate = Arc::clone(&self.gate);
            thread::Builder::new().name("counters fold".into()).spawn(move || {
                viewer_foundation::worker_priority::background_priority();
                run(
                    receive,
                    send,
                    #[cfg(any(test, feature = "test-support"))]
                    gate,
                )
            })?;
            (self.jobs, self.made) = (Some(jobs), Some(made));
        }
        let mut copies = Box::<Mirrors>::default();
        for table in TABLES.iter().chain(&action::TABLES) {
            if let Some(mirror) = mirrors.get(*table) {
                copies.insert(Box::new(mirror.clone()));
            }
        }
        self.spec.vision_new.store(false, Ordering::Release);
        let model = Box::new(self.fresh());
        let vision = upstream.get::<VisionLayer>().filter(|_| self.spec.reads_layer);
        self.sight = vision.map(VisionLayer::sight_keys);
        let sight = vision.map(|vision| Box::new(vision.sight_copy()));
        self.relating = upstream.get::<Relations>().map(|relations| (Arc::clone(relations.index()), relations.wars().cloned()));
        self.viewing = Some(self.viewing());
        self.epoch += 1;
        self.owed = 0;
        self.products.compose(mirrors.get(TableId::Divisions).and_then(Mirror::head));
        self.waiting.clear();
        let relating = self.relating.clone();
        self.send(Job::Reset { epoch: self.epoch, model, mirrors: copies, sight, relating });
        self.exchange(true);
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        self.compose_with(mirrors, &Upstream::new(&[]))
    }

    fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
        self.products.applied(step);
    }

    fn wants_ahead(&self) -> bool {
        true
    }

    /// The steps the frame applied (and those built past it), kept until the frame's changes go.
    fn ahead(&mut self, steps: &[Arc<Step>]) {
        self.waiting.extend(steps.iter().cloned());
    }

    /// Its worker is given only steps applied, so it folded nothing void: the steps kept past `kept`
    /// go, and are handed again.
    fn void_ahead(&mut self, _mirrors: &Mirrors, kept: Option<Hour>) {
        self.waiting.retain(|step| kept.is_some_and(|kept| step.last() <= kept));
    }

    /// What the worker has made taken: while the step waits for nothing, a product not yet made
    /// leaves the layer at the hour of the one it has and joins the step after. This frame's steps
    /// and its vision's, relations' and view's changes then go to the worker in one job. A settle
    /// that waits ([`Settling::waits`]: a whole settle, or this product at the bound) waits for the
    /// product of its hour.
    fn settle_with(&mut self, _mirrors: &Mirrors, upstream: &Upstream<'_>, settling: Settling) -> io::Result<()> {
        let whole = settling.waits();
        self.exchange(false);
        let applied = self.products.applied_hour();
        let mut steps = Vec::new();
        while self.waiting.front().is_some_and(|step| applied.is_some_and(|applied| step.last() <= applied)) {
            steps.extend(self.waiting.pop_front());
        }
        let sight = upstream.get::<VisionLayer>().filter(|_| self.spec.reads_layer).and_then(|vision| vision.sight_change(upstream.changed::<VisionLayer>(), &mut self.sight));
        let relating = upstream.get::<Relations>().map(|relations| (Arc::clone(relations.index()), relations.wars().cloned()));
        let relations_new = !matches!((&relating, &self.relating), (Some((index, wars)), Some((held, held_wars))) if Arc::ptr_eq(index, held) && wars.as_ref().map(Arc::as_ptr) == held_wars.as_ref().map(Arc::as_ptr));
        let relating = if relations_new && relating.is_some() {
            self.relating = relating.clone();
            relating
        } else {
            None
        };
        let viewing = Some(self.viewing()).filter(|viewing| self.viewing.as_ref() != Some(viewing));
        if viewing.is_some() {
            self.viewing = viewing;
        }
        let given = self.spec.vision_new.swap(false, Ordering::AcqRel).then(|| self.spec.perspective()).flatten();
        if !steps.is_empty() || sight.is_some() || relating.is_some() || viewing.is_some() || given.is_some() {
            self.send(Job::Frame { epoch: self.epoch, changes: Changes { stamp: applied, steps, sight, relating, viewing, given } });
        }
        self.exchange(whole);
        Ok(())
    }

    fn settle(&mut self, mirrors: &Mirrors, settling: Settling) -> io::Result<()> {
        self.settle_with(mirrors, &Upstream::new(&[]), settling)
    }


    fn carried(&self) -> Carried {
        // Every mark is the worker's: published, not carried.
        Carried::default()
    }

    /// Current once the product of the hour applied is taken; while a step is with the worker, the
    /// hour of the product it has.
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

/// What the window's counters layer saw of its worker, for the paced report.
pub fn summary(side: &crate::playhead_host::MapSide) -> serde_json::Value {
    let Some(layer) = side.parts().and_then(|(_, layers)| layers.iter().find_map(|layer| layer.any().downcast_ref::<Fed>())) else { return serde_json::Value::Null };
    let tally = layer.tally();
    serde_json::json!({
        "products": tally.products,
        "jobs": tally.jobs,
        "make_ms_mean": (tally.products > 0).then(|| tally.made_ns as f64 / tally.products as f64 / 1e6),
        "make_ms_worst": tally.made_ns_worst as f64 / 1e6,
        "scope": "The counters' worker as the frame saw it. jobs over products: the frames' jobs the worker made into one product when they queued while it worked. make_ms: the worker's time for a product, from the job received to the product sent. How often the layer stood behind the step's hour is in layers.standing, with every other layer's.",
    })
}
