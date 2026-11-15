//! The window's buildings: the slots, harbours and city damage ([`Buildings`]) made on a worker the
//! layer owns, with the product the scene installs. The worker keeps its own copies of the tables
//! they read (made at a compose, kept by applying each step as the frame's mirrors are) and of the
//! vision layer's bytes they read (the fog provinces', kept from what the vision layer names as
//! moved), settles every mark whole and makes the scene's product. The frame never waits for it
//! while playing: it installs the newest product that has arrived and shows the hour it is of.

use super::{Buildings, Declared, FromLayer, Inspected, Install, NOT_SLOTTED, READ, Seen, Upper};
use crate::{
    layers::{Budget, Carried, Fed as Products, Gpu, Inputs, Layer, Published, Scope, Scoped, Sight, Upstream, View, clock},
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
use viewer_foundation::map::perspective::{Choice, Vision};

/// What the scene gives the product: its building bank's binding and the city presentation's
/// thresholds.
type SceneInputs = ([u8; 32], Option<(i64, i64)>);

/// The vision layer's bytes the slots read (the fog provinces'), kept on the worker.
struct VisionCopy {
    state: (Option<u32>, bool, u64),
    refused: Option<String>,
    bytes: Vec<Option<u8>>,
    moved: Vec<u64>,
}

impl Seen for VisionCopy {
    fn ours(&self) -> u32 {
        self.state.0.unwrap_or_default()
    }

    fn byte(&self, province: u32) -> Option<u8> {
        self.bytes.get(province as usize).copied().flatten()
    }
}

impl Upper for VisionCopy {
    fn state(&self) -> (Option<u32>, bool, u64) {
        self.state
    }

    fn refused(&self) -> Option<String> {
        self.refused.clone()
    }

    fn seen(&self) -> &dyn Seen {
        self
    }

    fn moved(&self) -> &[u64] {
        &self.moved
    }
}

/// The vision layer as the frame last saw it, for the worker: gone, or its state and refusal with
/// the bytes of every fog province (`whole`, at a new sight epoch) or of those named as moved.
enum Seeing {
    Absent,
    Seen { state: (Option<u32>, bool, u64), refused: Option<String>, whole: bool, bytes: Vec<(u32, Option<u8>)>, moved: Vec<u64> },
}

enum Job {
    /// The buildings composed again from the tables' copies: the worker's own (made from `model` the
    /// first time), composed in place as the frame's layer is.
    Reset { epoch: u64, model: Option<Box<Buildings>>, mirrors: Box<Mirrors>, declared: Declared, seeing: Option<Seeing>, given: Option<(Choice, Arc<Vision>)>, scene: Option<SceneInputs> },
    /// One frame's view declared, steps in order, vision and scene; the product stamped `stamp`.
    Frame { epoch: u64, stamp: Option<Hour>, declared: Option<Declared>, steps: Vec<Arc<Step>>, seeing: Option<Seeing>, given: Option<(Choice, Arc<Vision>)>, scene: Option<SceneInputs> },
    /// What the proof compares, answered between products.
    Inspect(mpsc::Sender<Inspected>),
}

/// What the worker made for `answers` jobs, current at `stamp` (none: the compose's).
struct Made {
    epoch: u64,
    stamp: Option<Hour>,
    answers: usize,
    install: Option<Install>,
    /// The scene's inputs the install was made with.
    scene: Option<SceneInputs>,
    uploaded: bool,
    refused: Option<String>,
    error: Option<String>,
    warnings: Vec<String>,
    marked: [usize; 3],
    causes: [u64; super::CAUSES.len()],
    uploads: [u64; 5],
    upload_ns: [u64; 3],
    scoped: Scoped,
    held: usize,
}

/// The worker's buildings, the copies of what they read, and the scene's inputs once given.
struct Work {
    model: Box<Buildings>,
    mirrors: Box<Mirrors>,
    vision: Option<VisionCopy>,
    scene: Option<SceneInputs>,
}

impl Work {
    /// The view's declaration, a frame's steps applied to the copies and marked, then the vision
    /// and the scene's inputs, in the order the frame's layer takes them.
    fn take(&mut self, declared: Option<Declared>, steps: Vec<Arc<Step>>, seeing: Option<Seeing>, given: Option<(Choice, Arc<Vision>)>, scene: Option<SceneInputs>) {
        if let Some(declared) = declared {
            self.model.declare(declared);
        }
        for step in steps {
            self.mirrors.apply(&step);
            self.model.apply(&step, &self.mirrors);
        }
        match seeing {
            None => {}
            Some(Seeing::Absent) => self.vision = None,
            Some(Seeing::Seen { state, refused, whole, bytes, moved }) => {
                let size = self.model.fog_seen.len();
                let copy = match (&mut self.vision, whole) {
                    (Some(copy), false) => copy,
                    (held, _) => held.insert(VisionCopy { state, refused: None, bytes: vec![None; size], moved: Vec::new() }),
                };
                (copy.state, copy.refused) = (state, refused);
                for (province, byte) in bytes {
                    if let Some(held) = copy.bytes.get_mut(province as usize) {
                        *held = byte;
                    }
                }
                copy.moved.extend(moved);
            }
        }
        if let Some((choice, vision)) = given {
            self.model.set_vision(choice, vision);
        }
        if scene.is_some() {
            self.scene = scene;
        }
    }

    /// Composed whole from the copies, or settled whole, then the scene's product made.
    fn settle(&mut self, compose: bool) -> io::Result<Option<Install>> {
        let upper = self.vision.as_ref().map(|copy| copy as &dyn Upper);
        if compose {
            self.model.compose_upper(&self.mirrors, upper)?;
        } else {
            self.model.settle_dirty(&self.mirrors, &mut Budget::unlimited(), upper)?;
        }
        if let Some(copy) = &mut self.vision {
            copy.moved.clear();
        }
        let Some((bank, thresholds)) = self.scene else { return Ok(None) };
        self.model.note_thresholds(thresholds);
        self.model.make(bank, thresholds)
    }

    fn made(&mut self, epoch: u64, stamp: Option<Hour>, answers: usize, result: io::Result<Option<Install>>) -> Made {
        let model = &mut self.model;
        let held = model.held() + self.mirrors.iter().map(Mirror::bytes).sum::<usize>();
        let (install, error) = match result {
            Ok(install) => (install, None),
            Err(error) => (None, Some(error.to_string())),
        };
        Made {
            epoch,
            stamp,
            answers,
            install,
            scene: self.scene,
            uploaded: model.uploaded(),
            refused: model.refused.clone(),
            error,
            warnings: model.warnings(),
            marked: model.marked,
            causes: model.causes,
            uploads: model.uploads,
            upload_ns: model.upload_ns,
            scoped: std::mem::take(&mut model.scoped),
            held,
        }
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
            Job::Inspect(reply) => {
                if let Some(held) = &work {
                    let _ = reply.send(held.model.inspect());
                }
                continue;
            }
            Job::Reset { epoch: of, model, mirrors, declared, seeing, given, scene } => {
                epoch = of;
                let held = match (work.take(), model) {
                    (_, Some(model)) => Work { model, mirrors, vision: None, scene: None },
                    (Some(held), None) => Work { mirrors, ..held },
                    // Never sent: the first reset carries the buildings. The frame finds the worker stopped.
                    (None, None) => return,
                };
                let held = work.insert(held);
                held.take(Some(declared), Vec::new(), seeing, given, scene);
                let result = held.settle(true);
                held.made(epoch, None, 1, result)
            }
            // Jobs go in the order sent, each epoch's after its reset: one of an epoch composed again
            // is dropped by the frame.
            Job::Frame { epoch: of, .. } if of != epoch => continue,
            Job::Frame { stamp, declared, steps, seeing, given, scene, .. } => {
                let Some(held) = &mut work else { continue };
                held.take(declared, steps, seeing, given, scene);
                // The jobs queued meanwhile go into the same product, made for the newest hour.
                let (mut stamp, mut answers) = (stamp, 1);
                while let Ok(queued) = jobs.try_recv() {
                    match queued {
                        Job::Frame { epoch: of, stamp: at, declared, steps, seeing, given, scene } if of == epoch => {
                            held.take(declared, steps, seeing, given, scene);
                            (stamp, answers) = (at, answers + 1);
                        }
                        Job::Frame { .. } => {}
                        other => {
                            next = Some(other);
                            break;
                        }
                    }
                }
                let result = held.settle(false);
                held.made(epoch, stamp, answers, result)
            }
        };
        if made.send(product).is_err() {
            return;
        }
    }
}

/// The window's buildings layer: what its view declares, the frame's steps and the vision layer's
/// moves handed to its worker, and the worker's newest product installed.
pub(crate) struct BuildingsFed {
    /// The buildings as the view declares them: never composed or settled here.
    spec: Buildings,
    jobs: Option<mpsc::Sender<Job>>,
    made: Option<mpsc::Receiver<Made>>,
    epoch: u64,
    /// The worker holds its buildings (sent at the first compose).
    started: bool,
    /// Jobs of this epoch sent whose products have not been received.
    owed: usize,
    products: Products<Made>,
    /// The steps handed and not yet sent with a frame's job, in order.
    waiting: VecDeque<Arc<Step>>,
    /// The view's declaration as last taken, and whether it is new since the last job.
    declared: Option<Declared>,
    declared_new: bool,
    /// The vision layer's state and refusal as last sent; none: the next goes whole.
    seen_sent: Option<((Option<u32>, bool, u64), Option<String>)>,
    /// The vision given whole (the proof's) as last taken.
    given: Option<(Choice, Arc<Vision>)>,
    /// The scene's inputs as last seen at an upload.
    scene: Option<SceneInputs>,
    /// The newest product not installed yet, and whether a product dropped unshown started the
    /// scene's history cold.
    pending: Option<Install>,
    cold_owed: bool,
    uploaded: bool,
    refused: Option<String>,
    error: Option<String>,
    warnings: Vec<String>,
    marked: [usize; 3],
    causes: [u64; super::CAUSES.len()],
    uploads: [u64; 5],
    upload_ns: [u64; 3],
    install_ns: u64,
    scoped: Scoped,
    worker_held: usize,
    /// The worker stopped (it panicked): no buildings are installed from then on.
    stopped: bool,
}

impl BuildingsFed {
    /// The window's layer over `buildings`.
    pub(crate) fn new(buildings: Buildings) -> Self {
        Self {
            spec: buildings,
            jobs: None,
            made: None,
            epoch: 0,
            started: false,
            owed: 0,
            products: Products::default(),
            waiting: VecDeque::new(),
            declared: None,
            declared_new: false,
            seen_sent: None,
            given: None,
            scene: None,
            pending: None,
            cold_owed: false,
            uploaded: false,
            refused: Some("not composed yet".into()),
            error: None,
            warnings: Vec::new(),
            marked: [0; 3],
            causes: [0; super::CAUSES.len()],
            uploads: [0; 5],
            upload_ns: [0; 3],
            install_ns: 0,
            scoped: Scoped::default(),
            worker_held: 0,
            stopped: false,
        }
    }

    /// What the viewer sees at the head and the perspective it is for (the proof's).
    pub(crate) fn set_vision(&self, choice: Choice, vision: Arc<Vision>) {
        self.spec.set_vision(choice, vision);
    }

    pub(crate) fn perspective(&self) -> Option<&Choice> {
        self.given.as_ref().map(|(choice, _)| choice)
    }

    /// The hour's map-building warnings, as of the newest product.
    pub(crate) fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Rows, slots and provinces its worker marked, as of the newest product.
    pub(crate) fn marked(&self) -> [usize; 3] {
        self.marked
    }

    /// What the worker's buildings hold now, for the proof; none once it stopped.
    pub(crate) fn inspect(&self) -> Option<Inspected> {
        let (reply, answer) = mpsc::channel();
        self.jobs.as_ref()?.send(Job::Inspect(reply)).ok()?;
        answer.recv().ok()
    }

    /// Buildings as the view declares them, with the vision given, made from nothing.
    pub(crate) fn fresh(&self) -> Buildings {
        let fresh = self.spec.fresh();
        if let Some((choice, vision)) = &self.given {
            fresh.set_vision(choice.clone(), Arc::clone(vision));
        }
        fresh
    }

    pub(crate) fn figures(&self) -> serde_json::Value {
        let mut upload_ns = self.upload_ns;
        upload_ns[2] += self.install_ns;
        super::figures(&self.causes, &self.uploads, &upload_ns)
    }

    fn send(&mut self, job: Job) {
        if let Some(jobs) = &self.jobs
            && jobs.send(job).is_ok()
        {
            self.owed += 1;
        }
    }

    /// The vision layer's change since the last job, for the worker: whole at a new sight epoch (or
    /// the first), else the bytes of the fog provinces named as moved; none when nothing moved.
    fn seeing(&mut self, upstream: &Upstream<'_>) -> Option<Seeing> {
        if !self.spec.reads_layer {
            return None;
        }
        let Some(upper) = FromLayer::of(upstream) else { return self.seen_sent.take().map(|_| Seeing::Absent) };
        let (state, refused) = (upper.state(), upper.refused());
        let whole = self.seen_sent.as_ref().is_none_or(|(sent, _)| *sent != state);
        let fog = &self.spec.fog_seen;
        let moved: Vec<u64> = if whole { Vec::new() } else { upper.moved().iter().copied().filter(|&province| fog.get(province as usize).is_some_and(|&seen| seen != NOT_SLOTTED)).collect() };
        if !whole && moved.is_empty() && self.seen_sent.as_ref().is_some_and(|(_, sent)| *sent == refused) {
            return None;
        }
        let seen = upper.seen();
        let bytes = if whole { self.spec.fog_provinces.iter().map(|&province| (province, seen.byte(province))).collect() } else { moved.iter().map(|&province| (province as u32, seen.byte(province as u32))).collect() };
        self.seen_sent = Some((state, refused.clone()));
        Some(Seeing::Seen { state, refused, whole, bytes, moved })
    }

    /// A vision given whole since the last job.
    fn take_given(&mut self) -> Option<(Choice, Arc<Vision>)> {
        if !self.spec.fresh_vision.swap(false, Ordering::AcqRel) {
            return None;
        }
        let given = self.spec.given.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take()?;
        self.given = Some(given.clone());
        Some(given)
    }

    fn declared_now(&self) -> Declared {
        self.declared.unwrap_or(Declared { columns: self.spec.columns, fog: self.spec.fog })
    }

    /// Receives what the worker made and takes every product due, in order. With `wait` (a whole
    /// settle, a compose), until every job sent is answered; otherwise what has arrived.
    fn exchange(&mut self, wait: bool) -> io::Result<()> {
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
            if let Some((product, continues)) = made.install {
                // A product made with other scene inputs than the scene's now is never installed:
                // the worker makes another with these.
                if made.scene == self.scene {
                    if let Some((_, held)) = self.pending.replace((product, continues)) {
                        self.cold_owed |= !held;
                    }
                } else {
                    self.cold_owed |= !continues;
                }
            }
            (self.uploaded, self.refused, self.warnings, self.marked) = (made.uploaded, made.refused, made.warnings, made.marked);
            (self.causes, self.uploads, self.upload_ns, self.worker_held) = (made.causes, made.uploads, made.upload_ns, made.held);
            self.scoped.extend(made.scoped);
            if let Some(error) = made.error {
                self.error.get_or_insert(error);
            }
        }
        if self.stopped {
            self.pending = None;
        }
        match &self.error {
            Some(error) => Err(io::Error::other(format!("The buildings' worker could not settle: {error}"))),
            None => Ok(()),
        }
    }
}

impl Layer for BuildingsFed {
    fn name(&self) -> &'static str {
        "buildings"
    }

    fn placed(&mut self, view: &View) -> Option<u64> {
        self.spec.placed(view)
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        let result = self.spec.inputs(view, inputs);
        (self.declared, self.declared_new) = (Some(Declared { columns: self.spec.columns, fog: self.spec.fog }), true);
        result
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        self.compose_with(mirrors, &Upstream::new(&[]))
    }

    /// The worker composes its buildings again from the tables' copies (made here, off the frame
    /// thread) and the vision layer's bytes; its first product waited for.
    fn compose_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>) -> io::Result<()> {
        if self.jobs.is_none() {
            let (jobs, receive) = mpsc::channel();
            let (send, made) = mpsc::channel();
            thread::Builder::new().name("buildings".into()).spawn(move || {
                viewer_foundation::worker_priority::background_priority();
                run(receive, send)
            })?;
            (self.jobs, self.made) = (Some(jobs), Some(made));
        }
        let mut copies = Box::<Mirrors>::default();
        for table in READ {
            if let Some(mirror) = mirrors.get(table) {
                copies.insert(Box::new(mirror.clone()));
            }
        }
        let model = (!std::mem::replace(&mut self.started, true)).then(|| Box::new(self.spec.fresh()));
        self.seen_sent = None;
        let seeing = self.seeing(upstream);
        let given = self.take_given().or_else(|| self.given.clone());
        let declared = self.declared_now();
        self.declared_new = false;
        self.epoch += 1;
        self.owed = 0;
        self.products.compose(mirrors.get(TableId::Provinces).and_then(Mirror::head));
        self.waiting.clear();
        if let Some((_, continues)) = self.pending.take() {
            self.cold_owed |= !continues;
        }
        self.error = None;
        let scene = self.scene;
        self.send(Job::Reset { epoch: self.epoch, model, mirrors: copies, declared, seeing, given, scene });
        self.exchange(true)
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

    fn settle(&mut self, mirrors: &Mirrors, budget: &mut Budget) -> io::Result<()> {
        self.settle_with(mirrors, &Upstream::new(&[]), budget)
    }

    /// What the worker made taken, never waited for while playing; this frame's declaration, steps,
    /// vision moves and a vision given then go to the worker in one job. A whole settle waits for the
    /// product of its hour.
    fn settle_with(&mut self, _mirrors: &Mirrors, upstream: &Upstream<'_>, budget: &mut Budget) -> io::Result<()> {
        let whole = budget.unlimited_now();
        self.exchange(false)?;
        let applied = self.products.applied_hour();
        let mut steps = Vec::new();
        while self.waiting.front().is_some_and(|step| applied.is_some_and(|applied| step.last() <= applied)) {
            steps.extend(self.waiting.pop_front());
        }
        let declared = std::mem::take(&mut self.declared_new).then(|| self.declared_now());
        let seeing = self.seeing(upstream);
        let given = self.take_given();
        if declared.is_some() || !steps.is_empty() || seeing.is_some() || given.is_some() {
            self.send(Job::Frame { epoch: self.epoch, stamp: applied, declared, steps, seeing, given, scene: None });
        }
        self.exchange(whole)
    }

    /// The scene's inputs handed to the worker when they change (it makes no product before it has
    /// them); the newest product installed within the budget.
    fn upload(&mut self, gpu: &mut Gpu<'_>, budget: &mut Budget) -> io::Result<()> {
        if self.declared_now().columns.is_none() {
            return Ok(());
        }
        let thresholds = gpu.scene.city_damage_thresholds();
        match gpu.content.products.display_binding() {
            Ok(bank) if self.scene != Some((bank, thresholds)) => {
                self.scene = Some((bank, thresholds));
                let stamp = self.products.applied_hour();
                self.send(Job::Frame { epoch: self.epoch, stamp, declared: None, steps: Vec::new(), seeing: None, given: None, scene: Some((bank, thresholds)) });
            }
            Ok(_) => {}
            // As today: an hour to draw without the bank's binding fails the upload by name.
            Err(error) if self.refused.is_none() => return Err(error),
            Err(_) => {}
        }
        self.exchange(budget.unlimited_now())?;
        if self.refused.is_some() || self.pending.is_none() || !budget.left() {
            return Ok(());
        }
        let (product, continues) = self.pending.take().expect("a product is pending");
        let continues = continues && !std::mem::take(&mut self.cold_owed);
        let started = clock::now();
        super::install(gpu, (product, continues))?;
        self.install_ns += clock::now().duration_since(started).as_nanos() as u64;
        Ok(())
    }

    fn uploaded(&self) -> bool {
        self.pending.is_none() && self.uploaded
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

    fn scope(&mut self, sight: &Sight<'_>) -> Option<Scope> {
        Some(self.scoped.take(sight, None))
    }

    fn held(&self) -> usize {
        self.worker_held + self.waiting.capacity() * 8 + self.scoped.bytes()
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}
