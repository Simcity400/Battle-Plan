//! The Operatives map mode's intel icons (today `intel_icons::icons` per built hour) as a layer:
//! evaluated over rows at the playback head on the layer's own worker, never on the frame. While
//! playing it evaluates at the step that changes a table it reads (the rows' cadence), at once when
//! paused, after a seek and when a table arrives; an evaluation still running when newer rows come
//! is not queued behind: the newest rows are taken next and the ones passed over counted. It reads the tables the
//! content's scripts read, learned by discovery and kept for the content. At any other map mode it
//! declares nothing, has no reader, no worker and no icons. The camera's layout stays in the
//! overlay (`intel_icons::prepare`).

#[cfg(test)]
mod tests;

use super::{Budget, Carried, Gpu, Inputs, Layer, Published, View};
use crate::playhead_host::{
    Entry, MapSide,
    mirror::Mirrors,
    rows::{Cadence, Recording, RowReader, Rows, Stands},
};
use playhead::Step;
use replay_data::{
    Hour,
    rows::{TableId, tables},
};
use replay_frame::frame::{DecodedFrame, discovering};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io,
    sync::{Arc, OnceLock, mpsc},
    time::{Duration, Instant},
};
use viewer_foundation::map::{perspective, tables::core};
use viewer_raids::map::intel_icons::{self, IntelIcons};
use viewer_world::map::map_mode::{MapMode, Stock};

pub const NAME: &str = "intel icons";

/// The tables every evaluation reads first; the rest are learned. The Clock: its scripts compare
/// dates, so the icons are evaluated at every step.
const CORE: [TableId; 5] = [TableId::Countries, TableId::CountryLinks, TableId::Provinces, TableId::States, TableId::Clock];

/// How long an unlimited settle waits for the icons of the hour applied.
const CATCH_UP: Duration = Duration::from_secs(120);

/// The icons over one frame of rows, for a viewing choice: the recording's content in the window.
pub trait Evaluate: Send + Sync + 'static {
    fn icons(&self, frame: &DecodedFrame, choice: &perspective::Choice) -> IntelIcons;
}

impl Evaluate for crate::document::Content {
    fn icons(&self, frame: &DecodedFrame, choice: &perspective::Choice) -> IntelIcons {
        match ours(frame, choice) {
            Ok(ours) => intel_icons::icons(&self.icons.intel, &self.icons.raids, frame, ours, &self.names.localisation, &self.raid_hover.recorded_language),
            Err(why) => refused(frame, &why.to_string()),
        }
    }
}

/// The viewing country, as today's vision resolves it for the overlay ([`perspective::Vision::ours`]).
pub fn ours(frame: &DecodedFrame, choice: &perspective::Choice) -> io::Result<u32> {
    let links: BTreeMap<u32, u32> = core::<tables::CountryLinks>(frame)?.iter().map(|link| (link.id, link.canonical_slot)).collect();
    let tags: BTreeMap<u32, String> = core::<tables::Countries>(frame)?.iter().map(|country| (country.canonical_slot, String::from_utf8_lossy(&country.tag).into_owned())).collect();
    perspective::viewer(frame, &links, &tags, &choice.country).map(|(ours, _)| ours)
}

/// No icons, and why: the viewing country cannot be resolved at the hour.
fn refused(frame: &DecodedFrame, why: &str) -> IntelIcons {
    let mut icons = IntelIcons { built_for: None, ..IntelIcons::default() };
    icons.report.reasons.insert(format!("the viewing country cannot be resolved at hour {}: {why}", frame.hour), 1);
    icons
}

struct Job {
    frame: Arc<DecodedFrame>,
    choice: perspective::Choice,
    epoch: u64,
}

/// What the worker is handed: an evaluation, or something to free off the frame (old rows and old
/// icons can be the last reference to megabytes of row structs).
enum Work {
    Evaluate(Job),
    Free(Box<dyn Send>),
}

struct Done {
    /// None when the evaluation read a table not declared yet, named in `missed`.
    icons: Option<IntelIcons>,
    missed: Vec<TableId>,
    hour: Hour,
    /// The rows evaluated, held weakly.
    frame: std::sync::Weak<DecodedFrame>,
    ns: u64,
    epoch: u64,
}

/// The reader and the worker, while the icons are drawn.
struct Open {
    rows: Arc<Rows>,
    reader: RowReader,
    jobs: mpsc::Sender<Work>,
    done: mpsc::Receiver<Done>,
    /// An evaluation is running.
    busy: bool,
    /// The newest rows seen, and whether they were handed to the worker.
    seen: Option<Arc<DecodedFrame>>,
    handed: bool,
}

impl Open {
    fn new(entry: &Arc<Entry>, recording: Recording, tables: &[TableId], cadence: Cadence, evaluate: &Arc<dyn Evaluate>) -> io::Result<Self> {
        let rows = Rows::of(entry, recording)?;
        let reader = rows.reader(tables, cadence);
        let (jobs, receive) = mpsc::channel::<Work>();
        let (send, done) = mpsc::channel();
        let evaluate = Arc::clone(evaluate);
        // Not joined: closing drops the sender, and the thread ends after the job it holds.
        std::thread::Builder::new().name("intel icons".into()).spawn(move || {
            viewer_foundation::worker_priority::background_priority();
            for work in receive {
                let job = match work {
                    Work::Evaluate(job) => job,
                    Work::Free(held) => {
                        drop(held);
                        continue;
                    }
                };
                let started = Instant::now();
                let (icons, missed) = discovering(|| evaluate.icons(&job.frame, &job.choice));
                let (hour, frame) = (job.frame.hour, Arc::downgrade(&job.frame));
                drop(job.frame);
                let icons = missed.is_empty().then_some(icons);
                if send.send(Done { icons, missed, hour, frame, ns: started.elapsed().as_nanos() as u64, epoch: job.epoch }).is_err() {
                    return;
                }
            }
        })?;
        Ok(Self { rows, reader, jobs, done, busy: false, seen: None, handed: false })
    }

    /// Frees `held` on the worker.
    fn free(&self, held: impl Send + 'static) {
        let _ = self.jobs.send(Work::Free(Box::new(held)));
    }

    /// The newest rows seen from now on; the ones before are freed on the worker.
    fn see(&mut self, frame: Option<Arc<DecodedFrame>>) {
        if let Some(old) = std::mem::replace(&mut self.seen, frame) {
            self.free(old);
        }
    }
}

/// What the layer measured while drawn, each bounded by the values it takes, not by hours played.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    /// Evaluations by milliseconds, a tenth wide.
    pub evaluations: BTreeMap<u32, u64>,
    /// Frames by how many game hours the icons drawn are behind the hour the map applied.
    pub ages: BTreeMap<i32, u64>,
    /// Frames drawn in the mode before the first icons came (today's product shows then).
    pub loading: u64,
    /// Rows passed over: newer rows came before the worker was free for them.
    pub skipped: u64,
    /// Evaluations discarded because they read a table not declared yet.
    pub learning: u64,
    /// Results dropped because a reset or a new perspective came after their job.
    pub stale: u64,
    /// Times the reader and the worker were opened and closed.
    pub opened: u64,
    pub closed: u64,
}

/// Icons laid out by the window in place of the hour's, by microseconds on the frame thread, ten wide.
#[derive(Debug, Default)]
struct Installs(std::sync::Mutex<BTreeMap<u32, u64>>);

impl Stats {
    pub fn summary(&self) -> Value {
        let spread = |held: &BTreeMap<i32, u64>| {
            let total: u64 = held.values().sum();
            let at = |share: f64| {
                let wanted = (share * total as f64).ceil().max(1.0) as u64;
                let mut seen = 0;
                held.iter().find(|(_, count)| {
                    seen += **count;
                    seen >= wanted
                }).map(|(value, _)| *value)
            };
            json!({"count": total, "p50": at(0.5), "p99": at(0.99), "worst": held.keys().next_back()})
        };
        let evaluations: BTreeMap<i32, u64> = self.evaluations.iter().map(|(&tenths, &count)| (tenths as i32, count)).collect();
        let tenths = |value: Value| value.as_i64().map(|tenths| tenths as f64 / 10.0);
        let evaluation = spread(&evaluations);
        json!({
            "evaluations": evaluation["count"], "evaluation_ms_p50": tenths(evaluation["p50"].clone()), "evaluation_ms_p99": tenths(evaluation["p99"].clone()), "evaluation_ms_worst": tenths(evaluation["worst"].clone()),
            "age_hours": spread(&self.ages), "frames_loading": self.loading, "rows_skipped": self.skipped, "discarded_learning": self.learning, "dropped_stale": self.stale,
            "opened": self.opened, "closed": self.closed,
        })
    }
}

impl Installs {
    fn summary(&self) -> Value {
        let held = self.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let total: u64 = held.values().sum();
        let at = |share: f64| {
            let wanted = (share * total as f64).ceil().max(1.0) as u64;
            let mut seen = 0;
            held.iter().find(|(_, count)| {
                seen += **count;
                seen >= wanted
            }).map(|(tens, _)| f64::from(*tens) * 0.01)
        };
        json!({"count": total, "ms_p50": at(0.5), "ms_p99": at(0.99), "ms_worst": held.keys().next_back().map(|tens| f64::from(*tens) * 0.01)})
    }
}

/// The intel icons at the head.
pub struct Icons {
    evaluate: Arc<dyn Evaluate>,
    head: OnceLock<(Arc<Entry>, Recording)>,
    cadence: Cadence,
    /// The proof and the tests play the playhead themselves: rows follow the hour applied.
    drive: bool,
    shown: bool,
    choice: perspective::Choice,
    /// Learned by discovery; kept when closed, so it opens with them.
    tables: Vec<TableId>,
    open: Option<Open>,
    /// The newest icons and the hour of the rows they were evaluated over.
    icons: Option<(Arc<IntelIcons>, Hour)>,
    /// The rows the icons drawn were evaluated over, and the hour they stand at.
    stands: Option<Stands>,
    /// Bumped by a reset and a new perspective: results of earlier jobs are dropped.
    epoch: u64,
    applied: Option<Hour>,
    stats: Stats,
    installs: Installs,
}

impl Icons {
    /// The icons of `evaluate`'s content, over rows at the head of `head`'s playhead (or the one
    /// [`Self::set_head`] gives).
    pub fn new(evaluate: Arc<dyn Evaluate>, head: Option<(Arc<Entry>, Recording)>, cadence: Cadence) -> Self {
        let held = OnceLock::new();
        if let Some(head) = head {
            let _ = held.set(head);
        }
        Self { evaluate, head: held, cadence, drive: false, shown: false, choice: perspective::Choice::default(), tables: CORE.to_vec(), open: None, icons: None, stands: None, epoch: 0, applied: None, stats: Stats::default(), installs: Installs::default() }
    }

    /// The playhead it reads rows of, for a caller that makes the layer before the playhead; the
    /// first given stands.
    pub fn set_head(&self, entry: &Arc<Entry>, recording: Recording) {
        let _ = self.head.set((Arc::clone(entry), recording));
    }

    /// Rows follow the hour the map applied, for a caller that plays the playhead itself.
    pub fn drive(&mut self) {
        self.drive = true;
    }

    /// The newest icons and the hour of the rows they are of; none while loading or not drawn.
    pub fn icons(&self) -> Option<&(Arc<IntelIcons>, Hour)> {
        self.icons.as_ref()
    }

    pub fn tables(&self) -> &[TableId] {
        &self.tables
    }

    pub fn rows(&self) -> Option<&Arc<Rows>> {
        self.open.as_ref().map(|open| &open.rows)
    }

    pub fn stats(&self) -> &Stats {
        &self.stats
    }

    /// Whether a reader and a worker are open.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Evaluates `frame` here, as the worker does: the proof's check that the icons drawn are those
    /// of the newest rows.
    pub fn evaluate(&self, frame: &DecodedFrame) -> IntelIcons {
        discovering(|| self.evaluate.icons(frame, &self.choice)).0
    }

    /// The rows the reader holds now; none while a table loads.
    pub fn newest_rows(&self) -> Option<Arc<DecodedFrame>> {
        self.open.as_ref().and_then(|open| open.reader.frame())
    }

    fn close(&mut self) {
        let icons = self.icons.take();
        if let Some(open) = self.open.take() {
            self.stats.closed += 1;
            // The last reader of a playhead's rows joins their worker when dropped: never here.
            let _ = std::thread::Builder::new().name("intel icons closing".into()).spawn(move || {
                viewer_foundation::worker_priority::background_priority();
                drop((open, icons));
            });
        }
        self.epoch += 1;
    }

    /// Sets the icons drawn; the ones before are freed on the worker.
    fn show(&mut self, icons: Option<(Arc<IntelIcons>, Hour)>) {
        if let Some(old) = std::mem::replace(&mut self.icons, icons) {
            match &self.open {
                Some(open) => open.free(old),
                None => drop(old),
            }
        }
    }

    /// Opens the reader and the worker when drawn and a playhead is given; a refusal is printed.
    fn ensure_open(&mut self) {
        if self.open.is_some() || !self.shown {
            return;
        }
        let Some((entry, recording)) = self.head.get() else { return };
        match Open::new(entry, *recording, &self.tables, self.cadence, &self.evaluate) {
            Ok(open) => {
                if self.drive
                    && let Some(hour) = self.applied
                {
                    open.rows.drive(hour);
                }
                self.open = Some(open);
                self.stats.opened += 1;
            }
            Err(error) => eprintln!("The {NAME} layer's reader is refused: {error}"),
        }
    }

    /// Takes the worker's finished evaluations; never waits.
    fn receive(&mut self) {
        while let Some(done) = self.open.as_ref().and_then(|open| open.done.try_recv().ok()) {
            let _ = self.take(done);
        }
    }

    /// Whether it was discarded to learn a table.
    fn take(&mut self, done: Done) -> bool {
        let Some(open) = &mut self.open else { return false };
        open.busy = false;
        if done.epoch != self.epoch {
            self.stats.stale += 1;
            return false;
        }
        *self.stats.evaluations.entry((done.ns / 100_000) as u32).or_default() += 1;
        match done.icons {
            Some(icons) => {
                self.show(Some((Arc::new(icons), done.hour)));
                self.stands = Some(Stands::at(done.frame, done.hour));
            }
            None => {
                self.stats.learning += 1;
                self.tables.extend(done.missed);
                self.tables.sort_unstable_by_key(|table| table.index());
                self.tables.dedup();
                open.reader.declare(&self.tables);
                open.see(None);
                return true;
            }
        }
        false
    }

    /// Hands the newest rows to the worker when it is free; rows that a newer frame replaces before
    /// that are counted as passed over.
    fn offer(&mut self) {
        let Some(open) = &mut self.open else { return };
        if let Some(frame) = open.reader.frame()
            && !open.seen.as_ref().is_some_and(|seen| Arc::ptr_eq(seen, &frame))
        {
            if open.seen.is_some() && !open.handed {
                self.stats.skipped += 1;
            }
            open.see(Some(frame));
            open.handed = false;
        }
        if open.busy || open.handed {
            return;
        }
        let Some(frame) = open.seen.clone() else { return };
        if open.jobs.send(Work::Evaluate(Job { frame, choice: self.choice.clone(), epoch: self.epoch })).is_ok() {
            open.busy = true;
            open.handed = true;
        }
    }

    /// Waits until the icons are of the hour applied: an unlimited settle (the proof's, the tests').
    /// Gives up when the rows are already past that hour, and when a table was just learned: it
    /// joins at a step past the head, which never comes while the head is held.
    fn catch_up(&mut self) -> io::Result<()> {
        let Some(hour) = self.applied else { return Ok(()) };
        let until = Instant::now() + CATCH_UP;
        loop {
            if self.icons.as_ref().is_some_and(|(_, at)| *at == hour) && !self.open.as_ref().is_some_and(|open| open.busy) {
                return Ok(());
            }
            let Some(open) = &mut self.open else { return Ok(()) };
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(io::Error::other(format!("The {NAME} layer did not reach hour {hour} in {CATCH_UP:?}")));
            }
            if open.busy {
                let Ok(done) = open.done.recv_timeout(left) else {
                    return Err(io::Error::other(format!("The {NAME} layer's worker did not answer for hour {hour}")));
                };
                if self.take(done) {
                    return Ok(());
                }
                continue;
            }
            if self.drive {
                open.rows.drive(hour);
            }
            if open.reader.frame().is_none() {
                // Its tables are joining, at a step past the head.
                return Ok(());
            }
            // No evaluation running: it has finished with the rows it took, so the hour's come.
            open.reader.ready();
            let frame = match open.reader.wait_through(hour, left) {
                Ok(frame) => frame,
                Err(error) => return Err(io::Error::other(format!("The {NAME} layer's rows: {error}"))),
            };
            // Rows past the hour, or the rows seen standing at it.
            if frame.hour > hour || open.seen.as_ref().is_some_and(|seen| Arc::ptr_eq(seen, &frame)) {
                return Ok(());
            }
            open.see(Some(frame));
            open.handed = false;
            self.offer();
        }
    }

    /// The bytes the icons hold: states, entries, writes and their text, tooltips' text.
    fn icon_bytes(icons: &IntelIcons) -> usize {
        use intel_icons::Write;
        let mut bytes = std::mem::size_of::<IntelIcons>() + icons.states.capacity() * std::mem::size_of::<intel_icons::StateIcon>();
        for state in &icons.states {
            bytes += state.entries.capacity() * std::mem::size_of::<intel_icons::Entry>();
            for entry in &state.entries {
                bytes += entry.writes.capacity() * std::mem::size_of::<Write>();
                bytes += entry.writes.iter().map(|write| match write {
                    Write::Sprite(_, text) | Write::Portrait(_, text) => text.capacity(),
                    Write::Text(_, text) | Write::Withheld(_, text) => text.capacity(),
                    _ => 0,
                }).sum::<usize>();
                let tooltip = &entry.tooltip;
                bytes += std::mem::size_of_val(&**tooltip) + tooltip.title.capacity() + tooltip.body.capacity() + tooltip.refused.keys().map(String::capacity).sum::<usize>();
            }
        }
        bytes + icons.report.reasons.keys().map(String::capacity).sum::<usize>()
    }
}

impl Layer for Icons {
    fn name(&self) -> &'static str {
        NAME
    }

    fn placed(&mut self, _view: &crate::layers::View) -> Option<u64> {
        Some(0)
    }

    /// Drawn in the Operatives mode only: Countries' keys there, so a reset tells it the hour (its
    /// own tables come through rows at the head); nothing at any other mode.
    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        let shown = view.map_mode == MapMode::Stock(Stock::Operatives);
        if shown && view.perspective != self.choice {
            self.choice = view.perspective.clone();
            self.show(None);
            self.epoch += 1;
            if let Some(open) = &mut self.open {
                open.handed = false;
            }
        }
        self.shown = shown;
        if !shown {
            self.close();
            return Ok(());
        }
        inputs.table(TableId::Countries, true)
    }

    fn compose(&mut self, mirrors: &Mirrors) -> io::Result<()> {
        self.applied = mirrors.get(TableId::Countries).and_then(|mirror| mirror.head()).or(self.applied);
        self.icons = None;
        self.epoch += 1;
        if let Some(open) = &mut self.open {
            open.handed = false;
            if self.drive
                && let Some(hour) = self.applied
            {
                open.rows.drive(hour);
            }
        }
        Ok(())
    }

    fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
        self.applied = Some(step.last());
        if self.drive
            && let Some(open) = &self.open
        {
            open.rows.drive(step.last());
        }
    }

    fn settle(&mut self, _mirrors: &Mirrors, budget: &mut Budget) -> io::Result<()> {
        if !self.shown {
            return Ok(());
        }
        self.ensure_open();
        self.receive();
        self.offer();
        // No evaluation running: it has finished with the rows it took.
        if let Some(open) = self.open.as_ref().filter(|open| !open.busy) {
            open.reader.ready();
        }
        if budget.unlimited_now() {
            self.catch_up()?;
        }
        if let Some(open) = &self.open {
            if let Some(stands) = &mut self.stands {
                stands.look(|frame| open.reader.through(frame));
            }
        }
        match (&self.icons, self.applied) {
            (Some((_, at)), Some(applied)) => *self.stats.ages.entry(applied - at).or_default() += 1,
            (None, _) => self.stats.loading += 1,
            _ => {}
        }
        Ok(())
    }

    fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }

    /// Nothing: the icons are a step of rows or more behind by design, and their age is measured
    /// on its own ([`Stats::ages`]) rather than holding the map's drawn hour back.
    fn carried(&self) -> Carried {
        Carried::default()
    }

    fn published(&self) -> Published {
        self.icons.as_ref().map_or(Published::Current, |(_, hour)| Published::Cadence(self.stands.as_ref().map_or(*hour, |stands| stands.hour().max(*hour))))
    }

    fn held(&self) -> usize {
        self.icons.as_ref().map_or(0, |(icons, _)| Self::icon_bytes(icons)) + self.tables.capacity() * std::mem::size_of::<TableId>()
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The icons the map's layer draws, when it draws them.
pub fn drawn(side: &MapSide) -> Option<Arc<IntelIcons>> {
    if !side.draws(NAME) {
        return None;
    }
    let layer = side.parts()?.1.iter().find_map(|layer| layer.any().downcast_ref::<Icons>())?;
    layer.icons().map(|(icons, _)| Arc::clone(icons))
}

/// The window laid the layer's icons out in place of the hour's, in `spent`.
pub fn installed(side: &MapSide, spent: Duration) {
    if let Some(layer) = side.parts().and_then(|(_, layers)| layers.iter().find_map(|layer| layer.any().downcast_ref::<Icons>())) {
        *layer.installs.0.lock().unwrap_or_else(std::sync::PoisonError::into_inner).entry((spent.as_micros() / 10) as u32).or_default() += 1;
    }
}

/// The layer's figures for a report: what it measured, and what its rows hold.
pub fn summary(side: &MapSide) -> Value {
    let Some(layer) = side.parts().and_then(|(_, layers)| layers.iter().find_map(|layer| layer.any().downcast_ref::<Icons>())) else { return Value::Null };
    let held = layer.rows().map(|rows| rows.held());
    json!({
        "stats": layer.stats().summary(),
        "installs": layer.installs.summary(),
        "tables": layer.tables().iter().map(|table| format!("{table:?}")).collect::<Vec<_>>(),
        "icons_bytes": layer.held(),
        "rows_mirrors_bytes": held.as_ref().map(|held| held.mirrors.iter().map(|(_, bytes)| bytes).sum::<usize>()),
        "rows_bytes": held.as_ref().map(|held| held.rows.iter().map(|(_, bytes)| bytes).sum::<usize>()),
        "open": layer.is_open(),
    })
}
