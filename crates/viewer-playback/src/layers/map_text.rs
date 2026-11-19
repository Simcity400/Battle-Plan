//! The map text layer: the viewing country's floating text and critical bombardment explosions
//! from the recorded map events, made by today's maker ([`Prepared::read`]) and handed to the scene
//! in order, each interval once.
//!
//! The frame thread takes each step's interval as that step left it, from a copy of its own of the
//! one row stepped with each step (the map's mirror has every step of the frame applied before
//! the layers apply any), and, for an interval with
//! no event, makes it in place: there is nothing to decide. An interval holding events goes to the
//! layer's own worker ([`worker`]); the worker holds the tables the events' context and the
//! viewer's sight read, stepped to the end of the interval's own step, and makes it there: events
//! that read a province's fog read that hour's, never the hour the vision layer settled at. The frame presents finished intervals in order, a frame or two after their step.
//!
//! MapEvents is one row rewritten every hour, not a log: a step hands over its last hour's
//! interval only. A step of one hour presents every interval in turn; a longer step presents its
//! last hour's, and the scene's consumer names the intervals in between as a discontinuity.

#[cfg(test)]
mod tests;

pub mod proof;
mod worker;

use super::{Carried, Settling, Gpu, Inputs, Layer, Published, Upstream, View, vision::Vision};
use crate::playhead_host::{
    Entry,
    mirror::{Mirror, Mirrors},
    views::{Stepped, view},
};
use installed_content_localisation::localisation::Localisation;
use playhead::{Step, changes::Changes};
use replay_columnar::{manifest::Status, state::Kind};
use replay_data::{
    BranchId, Hour,
    rows::{MapEventPayload, TableId, tables},
};
use replay_frame::frame::{DecodedFrame, Metrics, SharedView, TableView, not_declared};
use std::{
    collections::VecDeque,
    io,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};
use viewer_foundation::map::{
    perspective::{Parts, VISION_TABLES, Vision as Seen},
    presentation::Continuity,
    visibility::FogContent,
};
use viewer_world::map::{
    animated_map_text::events::{self, Prepared},
    strike_effects::Impacts,
};

/// The tables today's maker reads: the interval first (from the map's mirror), then what its
/// context reads for an event (the worker's own, stepped with the map).
pub const TABLES: [TableId; 12] = [
    TableId::MapEvents,
    TableId::CountryLinks,
    TableId::LocalPlayer,
    TableId::Countries,
    TableId::CountryVisibility,
    TableId::LexicalTokens,
    TableId::ReservedCountryNaming,
    TableId::FactionMembers,
    TableId::CountryCounterInputs,
    TableId::WarRelations,
    TableId::RelationLinks,
    TableId::Subjects,
];

/// The tables the worker steps: the events' context, and what the viewer's sight of a province
/// reads at the interval's own hour.
fn worker_tables() -> Vec<TableId> {
    let mut tables: Vec<TableId> = TABLES[1..].iter().chain(VISION_TABLES).copied().collect();
    tables.sort_unstable();
    tables.dedup();
    tables
}

/// What today's maker takes besides the rows: the building types, the localisation, where a
/// province's explosion stands, and the fog content a province's sight is derived with.
pub trait Source: Send + Sync {
    fn text(&self) -> &events::Content;
    fn localisation(&self) -> &Localisation;
    fn place(&self, province: u32) -> Option<([f32; 3], [f32; 4])>;
    fn fog(&self) -> &Result<FogContent, String>;
}

impl Source for crate::document::Content {
    fn text(&self) -> &events::Content {
        &self.products.map_text
    }

    fn localisation(&self) -> &Localisation {
        &self.products.localisation
    }

    fn place(&self, province: u32) -> Option<([f32; 3], [f32; 4])> {
        self.products.impact_place(province)
    }

    fn fog(&self) -> &Result<FogContent, String> {
        &self.products.fog
    }
}

/// What the layer hands the scene, in order: an hour's interval, or why the hour has none.
#[derive(Clone, Debug)]
pub enum Presented {
    Hour { at: (BranchId, Hour), seek: bool, prepared: Arc<Prepared> },
    Missing { at: (BranchId, Hour), seek: bool, reason: String },
}

impl Presented {
    pub fn at(&self) -> (BranchId, Hour) {
        match self {
            Self::Hour { at, .. } | Self::Missing { at, .. } => *at,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn seek(&self) -> bool {
        match self {
            Self::Hour { seek, .. } | Self::Missing { seek, .. } => *seek,
        }
    }

    /// The interval as the display operand compares it, or the reason the hour has none.
    pub fn shown(&self) -> Result<map_display_data::MapText, &str> {
        match self {
            Self::Hour { prepared, .. } => Ok(prepared.display()),
            Self::Missing { reason, .. } => Err(reason),
        }
    }
}

/// A step the head took whose interval is still to hand on: its hour, whether it was sought, and
/// the interval as the step left it (none in a gap).
#[derive(Clone, Debug)]
struct Due {
    hour: Hour,
    seek: bool,
    interval: Option<SharedView>,
    /// The playhead's seek of its step.
    epoch: u64,
}

/// An interval in presentation order: made, or on the worker under its serial and its seek.
#[derive(Debug)]
enum Slot {
    Ready(Presented),
    Pending { serial: u64, epoch: u64 },
}

/// How long an unlimited settle (a compose, the proof's and the stepped harness's settles) waits for
/// the worker's intervals before it names the wait.
const WAIT: Duration = Duration::from_secs(120);

const GAP: &str = "map text presented hour is unavailable: the hour is a gap in the recording";

pub struct MapText {
    source: Arc<dyn Source>,
    branch: BranchId,
    /// MapEvents' column kinds, once declared.
    kinds: Vec<Kind>,
    due: VecDeque<Due>,
    /// The last hour applied (a compose's head): every interval through it is due or handed on.
    applied: Option<Hour>,
    /// The playhead's seek of the steps applied.
    epoch: u64,
    /// MapEvents' one row as the steps applied so far left it: the map's mirror stands at the
    /// frame's last step when the host has the layer apply each of a frame's steps.
    own: Mirrors,
    /// MapEvents' status as last taken: a change of it alone is a new interval.
    status: Option<Option<Status>>,
    /// The newest interval made with every one before it: the head's once nothing is pending.
    last: Option<Presented>,
    /// Made or on the worker, not yet handed to the scene, in order.
    queue: VecDeque<Slot>,
    /// The hour of the interval last handed to the scene.
    presented: Option<Hour>,
    order: Vec<(u128, u32)>,
    frame: u64,
    /// The playhead it was attached to, and the context tables the worker steps (declared at
    /// once, so they come with the next seek's reset) until the worker takes them; why not, when
    /// they could not be declared.
    rows: Mutex<Option<(Arc<Entry>, Option<Result<Stepped, String>>)>>,
    worker: Option<worker::Worker>,
    /// Steps that brought a new interval; intervals made in place (none holding an event);
    /// intervals made on the worker.
    marked: [u64; 3],
}

impl MapText {
    pub fn new(source: Arc<dyn Source>, branch: BranchId) -> Self {
        Self {
            source,
            branch,
            kinds: Vec::new(),
            due: VecDeque::new(),
            applied: None,
            epoch: 0,
            own: Mirrors::default(),
            status: None,
            last: None,
            queue: VecDeque::new(),
            presented: None,
            order: Vec::new(),
            frame: 0,
            rows: Mutex::new(None),
            worker: None,
            marked: [0; 3],
        }
    }

    /// Reads the events' context from `entry`'s playhead from now on, through tables of its own.
    pub fn attach(&self, entry: Arc<Entry>) {
        let stepped = Stepped::new(&entry, &worker_tables()).map_err(|error| format!("map text context: its tables cannot be declared: {error}"));
        *self.rows.lock().unwrap_or_else(PoisonError::into_inner) = Some((entry, Some(stepped)));
    }

    pub fn with_rows(self, entry: Arc<Entry>) -> Self {
        self.attach(entry);
        self
    }

    pub fn fresh(&self) -> Self {
        let fresh = Self { kinds: self.kinds.clone(), ..Self::new(Arc::clone(&self.source), self.branch) };
        if let Some((entry, _)) = &*self.rows.lock().unwrap_or_else(PoisonError::into_inner) {
            fresh.attach(Arc::clone(entry));
        }
        fresh
    }

    /// The interval made at the head; none before the first.
    pub fn product(&self) -> Option<&Presented> {
        self.last.as_ref()
    }

    /// The intervals made and not yet handed to the scene, in order, up to the first on the worker.
    #[cfg(any(test, feature = "test-support"))]
    pub fn pending(&self) -> Vec<&Presented> {
        self.queue
            .iter()
            .map_while(|slot| match slot {
                Slot::Ready(presented) => Some(presented),
                Slot::Pending { .. } => None,
            })
            .collect()
    }

    /// Whether intervals are on the worker.
    #[cfg(any(test, feature = "test-support"))]
    pub fn waiting(&self) -> bool {
        self.queue.iter().any(|slot| matches!(slot, Slot::Pending { .. }))
    }

    /// Its worker's liveness, for a test that drops the layer.
    #[cfg(any(test, feature = "test-support"))]
    pub fn worker_alive(&self) -> Option<Arc<std::sync::atomic::AtomicBool>> {
        self.worker.as_ref().map(worker::Worker::alive)
    }

    /// Steps that brought a new interval; intervals made in place, none holding an event;
    /// intervals made on the worker.
    pub fn marked(&self) -> [u64; 3] {
        self.marked
    }

    /// The interval's rows as the step left them, as today's frame reader makes them: one row.
    fn view(&mut self, mirror: &Mirror) -> SharedView {
        let (made, _) = view::make(TableId::MapEvents, mirror, (&self.kinds, u64::MAX), false, &mut self.order);
        let sampled = if matches!(made, TableView::Complete(_)) { mirror.sampled() } else { None };
        SharedView::from(made).with_sampled_at(sampled)
    }

    fn mark(&mut self, hour: Hour, seek: bool, interval: Option<SharedView>) {
        self.marked[0] += 1;
        self.due.push_back(Due { hour, seek, interval, epoch: self.epoch });
    }

    /// The worker, started once: with the context tables when attached, refusing them by name
    /// otherwise.
    fn worker(&mut self) -> io::Result<&mut worker::Worker> {
        if self.worker.is_none() {
            let stepped = self.rows.lock().unwrap_or_else(PoisonError::into_inner).as_mut().and_then(|(_, stepped)| stepped.take());
            let stepped = stepped.unwrap_or_else(|| Err("map text context: no rows at the head are attached to the layer".to_owned()));
            self.worker = Some(worker::Worker::start(Arc::clone(&self.source), self.branch, stepped)?);
        }
        Ok(self.worker.as_mut().expect("started"))
    }

    /// One due interval handed on: made in place when the hour has none or it holds no event,
    /// otherwise sent to the worker for the vision of the interval's own hour (its viewer and its
    /// fog), which the worker derives from that hour's rows.
    fn hand_on(&mut self, due: Due, vision: Option<&Vision>) -> io::Result<()> {
        let at = (self.branch, due.hour);
        let seek = due.seek;
        let missing = |reason: String| Slot::Ready(Presented::Missing { at, seek, reason });
        let slot = match (due.interval, vision) {
            (None, _) => missing(GAP.to_owned()),
            (_, None) => missing("map text presented hour failed: the vision layer is not drawn".to_owned()),
            (Some(interval), Some(vision)) => {
                let mut frame = DecodedFrame { branch: self.branch, hour: due.hour, tables: std::array::from_fn(|index| not_declared(TableId::ALL[index])), metrics: Metrics::default() };
                frame.tables[TableId::MapEvents.index()] = interval.clone();
                let provinces = read_provinces(&frame);
                match (provinces, vision.refused()) {
                    (Some(_), _) => Slot::Pending { serial: self.worker()?.send(worker::Job { at, seek, epoch: due.epoch, interval, choice: vision.choice().clone() }), epoch: due.epoch },
                    (None, Some(refused)) => missing(format!("map text presented hour failed: {refused}")),
                    (None, None) => {
                        self.marked[1] += 1;
                        let seen = Seen::from_parts(Parts { ours: vision.ours().unwrap_or_default(), ..Parts::default() });
                        let source = &*self.source;
                        Slot::Ready(Presented::Hour { at, seek, prepared: Arc::new(Prepared::read(&frame, &seen, source.text(), source.localisation(), |province| source.place(province))) })
                    }
                }
            }
        };
        self.queue.push_back(slot);
        Ok(())
    }

    /// The playhead's entry the context is read from, when attached.
    fn entry(&self) -> Option<Arc<crate::playhead_host::Entry>> {
        self.rows.lock().unwrap_or_else(PoisonError::into_inner).as_ref().map(|(entry, _)| Arc::clone(entry))
    }

    /// The worker's finished intervals put in their places, and those of a seek since superseded
    /// let go unmade; with `wait`, until none is pending. Waiting holds the frame, so it stands the
    /// head: context tables declared since the seek (a layer composed anew) land past the head while
    /// the clock plays, and are read at it instead.
    fn take(&mut self, wait: Option<Duration>) -> io::Result<()> {
        let entry = self.entry();
        if let Some(worker) = &mut self.worker {
            loop {
                for (serial, presented) in worker.take() {
                    self.marked[2] += 1;
                    if let Some(slot) = self.queue.iter_mut().find(|slot| matches!(slot, Slot::Pending { serial: held, .. } if *held == serial)) {
                        *slot = Slot::Ready(presented);
                    }
                }
                let newest = entry.as_ref().map(|entry| entry.playhead().epoch());
                if let Some(newest) = newest {
                    self.queue.retain(|slot| !matches!(slot, Slot::Pending { epoch, .. } if *epoch < newest));
                }
                let pending = self.queue.iter().any(|slot| matches!(slot, Slot::Pending { .. }));
                let Some(limit) = wait.filter(|_| pending) else { break };
                let look = || match &entry {
                    Some(entry) => {
                        entry.playhead().stand();
                        Some(entry.playhead().epoch()) == newest
                    }
                    None => true,
                };
                if !worker.wait(limit, look) {
                    return Err(io::Error::other(format!("The map text layer's worker made no interval in {limit:?}")));
                }
            }
        }
        if let Some(Slot::Ready(presented)) = self.queue.iter().take_while(|slot| matches!(slot, Slot::Ready(_))).last() {
            self.last = Some(presented.clone());
        }
        Ok(())
    }

    pub fn differ(&self, other: &Self) -> (u64, Option<String>) {
        differ(self.product(), other.product().map(Presented::shown).transpose().map_err(str::to_owned))
    }
}

/// The provinces an interval's events read the viewer's sight of; none when it holds no event or
/// cannot be read (today's maker then names why, with no event to decide).
fn read_provinces(frame: &DecodedFrame) -> Option<Vec<u32>> {
    let rows = viewer_foundation::table_access::rows::<tables::MapEvents>(frame).ok()?;
    let events: Vec<_> = rows.rows().iter().flat_map(|row| &row.events).collect();
    if events.is_empty() {
        return None;
    }
    Some(
        events
            .into_iter()
            .filter_map(|event| match &event.payload {
                MapEventPayload::FakeDivisionRevealed(unit) | MapEventPayload::RailwayGunOverrun(unit) => Some(unit.visibility_province),
                MapEventPayload::ParadropLanding(paradrop) => Some(paradrop.province),
                _ => None,
            })
            .collect(),
    )
}

/// An interval made at the head against another product: 0 or 1 differing, the difference described.
pub fn differ(ours: Option<&Presented>, theirs: Result<Option<map_display_data::MapText>, String>) -> (u64, Option<String>) {
    let ours = ours.map(Presented::shown).transpose().map_err(str::to_owned);
    if ours == theirs { (0, None) } else { (1, Some(format!("{ours:?} against {theirs:?}"))) }
}

/// Whether a step changed the table: its rows, cells or status.
fn touched(changes: &Changes) -> bool {
    changes.reset() || changes.renumber().is_some() || !changes.deleted().is_empty() || !changes.inserted().is_empty() || changes.columns().iter().any(|written| !written.is_empty())
}

impl Layer for MapText {
    fn name(&self) -> &'static str {
        "map_text"
    }

    fn placed(&mut self, _view: &crate::layers::View) -> Option<u64> {
        Some(0)
    }

    fn inputs(&mut self, _view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        inputs.after("vision");
        let table = TableId::MapEvents;
        inputs.table(table, true)?;
        let kinds = replay::native_columns::columns(table.schema())?;
        for column in 0..kinds.len() {
            inputs.index(table, column)?;
        }
        self.kinds = kinds;
        Ok(())
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        self.compose_with(mirrors, &Upstream::new(&[]))
    }

    /// The head's interval, sought, made before it returns (on the worker when it holds events);
    /// what was queued before the reset is dropped.
    fn compose_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>) -> io::Result<()> {
        self.queue.clear();
        self.due.clear();
        if let Some(worker) = &mut self.worker {
            worker.reset();
        }
        self.own = Mirrors::default();
        self.epoch = mirrors.epoch();
        let Some(mirror) = mirrors.get(TableId::MapEvents) else { return Ok(()) };
        self.own.insert(Box::new(mirror.clone()));
        let Some(hour) = mirror.head() else { return Ok(()) };
        self.applied = Some(hour);
        self.status = Some(mirror.status());
        let interval = mirror.status().is_some().then(|| self.view(mirror));
        let stepped = self.marked[0];
        self.mark(hour, true, interval);
        self.marked[0] = stepped;
        self.settle_with(mirrors, upstream, Settling::Whole)
    }

    fn apply(&mut self, step: &Step, mirrors: &Mirrors) {
        self.apply_with(step, mirrors, &Upstream::new(&[]));
    }

    /// A step that changed the interval takes it as that step left it: one row, from the layer's
    /// own copy stepped with each step (the map's mirror stands at the frame's last step).
    fn apply_with(&mut self, step: &Step, mirrors: &Mirrors, _upstream: &Upstream<'_>) {
        if self.kinds.is_empty() {
            return;
        }
        self.applied = Some(step.last());
        self.epoch = step.epoch();
        self.own.apply(step);
        if step.gap() {
            self.mark(step.last(), false, None);
            return;
        }
        let own = std::mem::take(&mut self.own);
        let mirror = own.get(TableId::MapEvents).or_else(|| mirrors.get(TableId::MapEvents));
        let Some(mirror) = mirror else {
            self.own = own;
            return;
        };
        let status = Some(mirror.status());
        let after_gap = match self.due.back() {
            Some(due) => due.interval.is_none(),
            None => matches!(&self.last, Some(Presented::Missing { reason, .. }) if reason == GAP),
        };
        let changed = step.table(TableId::MapEvents.index()).is_some_and(touched) || status != self.status;
        if changed || after_gap {
            self.status = status;
            let interval = self.view(mirror);
            self.mark(step.last(), false, Some(interval));
        }
        self.own = own;
    }

    fn settle(&mut self, mirrors: &Mirrors, settling: Settling) -> io::Result<()> {
        self.settle_with(mirrors, &Upstream::new(&[]), settling)
    }

    /// Hands on the due intervals within the settling, then joins the worker's within its budget.
    fn settle_with(&mut self, _mirrors: &Mirrors, upstream: &Upstream<'_>, settling: Settling) -> io::Result<()> {
        self.frame += 1;
        let entry = self.entry();
        // A seek since superseded abandoned these intervals' steps: no rows decide them.
        if let Some(newest) = entry.as_ref().map(|entry| entry.playhead().epoch()) {
            self.due.retain(|due| due.epoch >= newest);
        }
        let vision = upstream.get::<Vision>();
        while !self.due.is_empty() {
            let due = self.due.pop_front().expect("not empty");
            self.hand_on(due, vision)?;
        }
        // The worker's context steps through every hour whose interval is handed on: its reader
        // holds the playhead's steps until then.
        let through = self.due.front().map_or(self.applied.map(|hour| (hour, self.epoch)), |due| Some((due.hour - 1, due.epoch)));
        if let Some((hour, epoch)) = through
            && (entry.is_some() || self.worker.is_some())
        {
            self.worker()?.through(hour, epoch);
        }
        // Out of the step's budget, the intervals it holds stay pending and the step after joins
        // them; only a settle that waits however long it takes is refused by name.
        match self.take(Some(settling.waiting(WAIT))) {
            Err(error) if settling.strict() => Err(error),
            _ => Ok(()),
        }
    }

    /// The finished intervals in order through the scene's own input, up to the first still on the
    /// worker; the impacts are the strikes layer's. Its continuity is the consumer's, from the hour
    /// and whether it was sought.
    fn upload(&mut self, gpu: &mut Gpu<'_>) -> io::Result<()> {
        self.take(None)?;
        let content = gpu.content;
        let none = Impacts::default();
        while matches!(self.queue.front(), Some(Slot::Ready(_))) {
            let Some(Slot::Ready(presented)) = self.queue.pop_front() else { break };
            self.presented = Some(presented.at().1);
            match presented {
                Presented::Hour { at, seek, prepared } => gpu.scene.present_hours(at, seek, [(&none, &*prepared)], Continuity::Continue, |province| content.counters.anchor(province, 0), content.heights(), &content.icons.counters.gui),
                Presented::Missing { at, seek, reason } => gpu.scene.missing_map_text_events(at, seek, reason),
            }
        }
        Ok(())
    }

    fn carried(&self) -> Carried {
        Carried { entities: self.due.len() }
    }

    /// While the worker holds an interval: the hour of the newest made before it, or presented
    /// last; current once it holds none.
    fn published(&self) -> Published {
        let Some(first) = self.queue.iter().position(|slot| matches!(slot, Slot::Pending { .. })) else { return Published::Current };
        let before = self
            .queue
            .iter()
            .take(first)
            .filter_map(|slot| match slot {
                Slot::Ready(presented) => Some(presented.at().1),
                Slot::Pending { .. } => None,
            })
            .last();
        before.or(self.presented).map_or(Published::Nothing, Published::At)
    }

    fn held(&self) -> usize {
        self.kinds.capacity() + self.order.capacity() * size_of::<(u128, u32)>() + self.queue.capacity() * size_of::<Slot>() + self.due.capacity() * size_of::<Due>() + self.own.iter().map(Mirror::bytes).sum::<usize>()
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}
