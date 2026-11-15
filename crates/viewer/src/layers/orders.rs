//! The battle plan (`OrderField`) from rows at the head, built whole on the layer's own worker.
//!
//! A plan is one product, not a sum of orders: each order's geometry takes the overlap counters
//! the orders visited before it left (in the roots' drawing order), and each label is placed
//! against the labels before it. So it is made again whole when a table it reads changed, never
//! per order from columns; its tables include the divisions, which change every step in a war, so
//! it is made at the step that changes a table it reads while playing (paused or after a seek at
//! once), the steps that came during a remake taken together, off the
//! frame. Ordinary plans read only [`orders::memo_tables`], as today; invasions, active orders and
//! labelled paradrops read the whole hour (their triggers may read any table), so the rows they
//! read are learned as they read them and the plan is kept until those rows arrive.

#[cfg(test)]
mod tests;

use super::{Budget, Carried, Gpu, Inputs, Layer, Published, Upstream, View};
use crate::playhead_host::{
    mirror::{Mirror, Mirrors},
    rows::{Cadence, Newest, RowReader, Rows},
};
use playhead::Step;
use replay_data::{Hour, rows::TableId};
use replay_frame::frame::{DecodedFrame, discovering};
use std::{
    fmt::Debug,
    io,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    time::{Duration, Instant},
};
use viewer_counter_products::map::orders;
use viewer_foundation::map::{
    memo::{TableKey, project},
    name_draw::{Placements, WorldName},
    perspective::{Choice, Vision},
    ribbon_content::OrderField,
};

/// What builds a plan from a frame of rows: the recording's content in the viewer.
pub(crate) trait Plans: Send + Sync {
    fn build(&self, frame: &DecodedFrame, ours: u32, show_allied: bool) -> io::Result<OrderField>;
}

impl Plans for crate::document::Content {
    /// The memo tables alone unless the hour needs the whole frame, as today's plan.
    fn build(&self, frame: &DecodedFrame, ours: u32, show_allied: bool) -> io::Result<OrderField> {
        // Invasions' triggers read the recording's executable tables.
        engine_executable::executable_tables::scope(self.tables, || {
            let links = crate::document::links_of(frame)?;
            if orders::requires_full_frame(frame, &links, ours, show_allied, self.icons.fonts.arrow().is_some())? {
                self.battle_plan(frame, &links, ours, show_allied)
            } else {
                self.battle_plan(&project(frame, &orders::memo_tables(show_allied)), &links, ours, show_allied)
            }
        })
    }
}

/// Where two plans' labels part: the first label by key and its first differing field, with both
/// values. In order, since each draw names its label by index; the upload generation aside, as
/// every plan made takes a new one.
pub(crate) fn differ_labels(today: &Placements, layer: &Placements) -> Option<String> {
    let Placements { country, warnings, generation: _, near } = today;
    for (at, (theirs, ours)) in country.iter().zip(&layer.country).enumerate() {
        if theirs.key != ours.key {
            return Some(format!("label {at}: today {:?}, the layer {:?}", theirs.key, ours.key));
        }
        if let Some((field, theirs_value, ours_value)) = differ_name(theirs, ours) {
            return Some(format!("label {:?} (at {at}), {field}: today {theirs_value}, the layer {ours_value}", theirs.key));
        }
    }
    if country.len() != layer.country.len() {
        let extra = |names: &[WorldName]| names.get(country.len().min(layer.country.len())).map(|name| name.key.clone());
        return Some(format!("{} labels today, {} the layer; the first unmatched today {:?}, the layer {:?}", country.len(), layer.country.len(), extra(country), extra(&layer.country)));
    }
    if let Some((field, theirs, ours)) = each("warnings", warnings, &layer.warnings) {
        return Some(format!("{field}: today {theirs}, the layer {ours}"));
    }
    match (near, &layer.near) {
        (None, None) => None,
        (Some(theirs), Some(ours)) => differ_labels(theirs, ours).map(|part| format!("near: {part}")),
        _ => Some(format!("near: today {}, the layer {}", near.is_some(), layer.near.is_some())),
    }
}

fn differ_name(today: &WorldName, layer: &WorldName) -> Option<(String, String, String)> {
    let WorldName { key: _, strategic_provinces, text, pages, corners, uvs, aabb, wrapped, font_size, spaces, alpha } = today;
    whole("strategic_provinces", &strategic_provinces.is_some(), &layer.strategic_provinces.is_some())
        .or_else(|| each("strategic_provinces", strategic_provinces.as_deref().unwrap_or_default(), layer.strategic_provinces.as_deref().unwrap_or_default()))
        .or_else(|| whole("text", text, &layer.text))
        .or_else(|| each("pages", pages, &layer.pages))
        .or_else(|| each("corners", corners, &layer.corners))
        .or_else(|| each("uvs", uvs, &layer.uvs))
        .or_else(|| each("aabb", aabb, &layer.aabb))
        .or_else(|| whole("wrapped", wrapped, &layer.wrapped))
        .or_else(|| whole("font_size", font_size, &layer.font_size))
        .or_else(|| whole("spaces", spaces, &layer.spaces))
        .or_else(|| whole("alpha", alpha, &layer.alpha))
}

fn whole<T: Debug + ?Sized>(field: &str, today: &T, layer: &T) -> Option<(String, String, String)> {
    let (today, layer) = (format!("{today:?}"), format!("{layer:?}"));
    (today != layer).then(|| (field.to_owned(), today, layer))
}

fn each<T: Debug>(field: &str, today: &[T], layer: &[T]) -> Option<(String, String, String)> {
    if let Some(at) = today.iter().zip(layer).position(|(theirs, ours)| format!("{theirs:?}") != format!("{ours:?}")) {
        return Some((format!("{field}[{at}]"), format!("{:?}", today[at]), format!("{:?}", layer[at])));
    }
    (today.len() != layer.len()).then(|| (format!("{field} length"), today.len().to_string(), layer.len().to_string()))
}

struct Job {
    frame: Arc<DecodedFrame>,
    tables: Vec<TableId>,
    ours: u32,
    show_allied: bool,
}

struct Made {
    hour: Hour,
    /// The jobs it answers: the one built and those it passed over for it.
    jobs: usize,
    /// Tables the build read that the frame did not hold: the plan is not taken.
    missed: Vec<TableId>,
    plan: Result<OrderField, String>,
    /// The viewing country's own orders, which the battle plan's notes are stated for.
    own: Result<usize, String>,
    micros: u64,
}

/// The battle plan's limitation notes for `own` orders of the viewing country, in today's words.
fn plan_notes(own: Result<usize, String>) -> Vec<String> {
    match own {
        Ok(drawn) => {
            let mut out = viewer_foundation::map::tables::Limitations::default();
            out.note_battle_plan(drawn);
            out.into_lines()
        }
        Err(error) => vec![format!("Field ArmyOrderPlans: the viewing country's own orders cannot be counted at the playback head, so what the drawn battle plan cannot take from the recording is not stated: {error}")],
    }
}

fn spawn(plans: Arc<dyn Plans>) -> io::Result<(Sender<Job>, Receiver<Made>)> {
    let (jobs, receive) = mpsc::channel::<Job>();
    let (send, made) = mpsc::channel::<Made>();
    std::thread::Builder::new().name("battle plan".into()).spawn(move || {
        viewer_foundation::worker_priority::background_priority();
        while let Ok(mut job) = receive.recv() {
            let mut jobs = 1;
            while let Ok(newer) = receive.try_recv() {
                job = newer;
                jobs += 1;
            }
            let started = Instant::now();
            let ((plan, own), missed) = discovering(|| {
                let own = crate::document::links_of(&job.frame).and_then(|links| orders::own_order_count(&job.frame, &links, job.ours));
                (plans.build(&job.frame, job.ours, job.show_allied), own)
            });
            let missed = missed.into_iter().filter(|table| !job.tables.contains(table)).collect();
            let made = Made { hour: job.frame.hour, jobs, missed, plan: plan.map_err(|error| error.to_string()), own: own.map_err(|error| error.to_string()), micros: started.elapsed().as_micros() as u64 };
            if send.send(made).is_err() {
                return;
            }
        }
    })?;
    Ok((jobs, made))
}

pub(crate) struct Orders {
    plans: Arc<dyn Plans>,
    given: Mutex<Option<(Choice, Arc<Vision>)>>,
    /// The perspective and the viewing country's slot the plan is made for: all it reads of a
    /// vision.
    vision: Option<(Choice, u32)>,
    /// The rows at the head, which follow the head the map takes.
    rows: Mutex<Option<RowReader>>,
    worker: Option<(Sender<Job>, Receiver<Made>)>,
    /// The tables the reader is declared: the memo tables and those a whole-hour build read.
    tables: Vec<TableId>,
    learned: Vec<TableId>,
    /// Registered in the window: its vision is read from the vision layer before it.
    window: bool,
    /// Declared again for tables a build read: their resets come with a later step.
    relearning: bool,
    show_allied: bool,
    /// What the newest job was made from; a frame of the same tables, viewer and switch is not sent.
    sent: Option<(TableKey, u32, bool)>,
    /// The hour of the newest rows looked at; the plan is theirs once nothing is in flight.
    looked: Option<Hour>,
    /// The last hour the map's steps reached.
    head: Option<Hour>,
    in_flight: usize,
    /// The newest hour of the rows the plan it shows is the plan of: while a remake is on the
    /// worker or a table it learned is joining its rows, what it publishes.
    through: Option<Hour>,
    plan: Option<(Hour, Arc<OrderField>)>,
    /// The battle plan's limitation notes for the rows the plan was made from.
    notes: Vec<String>,
    refused: Option<String>,
    /// Plans made, rebuilt for tables learned, and their microseconds, since the layer was made.
    marked: [u64; 3],
}

impl Orders {
    pub(crate) fn new(plans: Arc<dyn Plans>) -> Self {
        Self {
            plans,
            given: Mutex::new(None),
            vision: None,
            rows: Mutex::new(None),
            worker: None,
            tables: orders::memo_tables(false),
            learned: Vec::new(),
            window: false,
            relearning: false,
            show_allied: false,
            sent: None,
            looked: None,
            head: None,
            in_flight: 0,
            through: None,
            plan: None,
            notes: Vec::new(),
            refused: None,
            marked: [0; 3],
        }
    }

    /// The rows at the head it builds from.
    pub(crate) fn attach(&self, rows: Arc<Rows>) {
        let reader = rows.reader(&self.tables, Cadence::Ready);
        *self.rows.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(reader);
    }

    /// Registered in the window: its vision is read from the vision layer before it.
    pub(crate) fn in_window(mut self) -> Self {
        self.window = true;
        self
    }

    /// What the viewer sees at the head: whose plan is drawn.
    pub(crate) fn set_vision(&self, choice: Choice, vision: Arc<Vision>) {
        *self.given.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((choice, vision));
    }

    pub(crate) fn perspective(&self) -> Option<&Choice> {
        self.vision.as_ref().map(|(choice, _)| choice)
    }

    pub(crate) fn show_allied(&self) -> bool {
        self.show_allied
    }

    /// The newest plan and the hour of the rows it was made from.
    pub(crate) fn plan(&self) -> Option<(Hour, &Arc<OrderField>)> {
        self.plan.as_ref().map(|(hour, plan)| (*hour, plan))
    }

    pub(crate) fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    /// The hour of the rows its plan (or refusal) stands for: none while a build is in flight or
    /// a table a build read is still joining its rows. A plan's own hour is its tables' last change.
    pub(crate) fn made_for(&self) -> Option<Hour> {
        (!self.relearning && self.in_flight == 0).then_some(self.looked).flatten()
    }

    /// The plan's refusals and missing inputs, worded as today's; loading by name until a plan is made.
    pub(crate) fn warnings(&self) -> Vec<String> {
        match (self.plan(), self.refused()) {
            (Some((_, plan)), _) => plan.report.warnings(),
            (None, Some(why)) => vec![format!("Battle plan: not drawn: {why}")],
            (None, None) => vec!["Battle plan: loading at the playback head".to_owned()],
        }
    }

    /// The battle plan's limitation notes in today's words, for the rows its plan was made from:
    /// stated only while the viewing country has orders.
    pub(crate) fn notes(&self) -> &[String] {
        &self.notes
    }

    pub(crate) fn marked(&self) -> [u64; 3] {
        self.marked
    }

    /// The plan made from the rows at `hour`, waiting for them and for the build (the previous
    /// plan when the build read tables not declared yet): for a caller that plays the playhead
    /// itself, never the window. Only rows standing at `hour` answer it, never rows past it.
    fn settle_at(&mut self, hour: Hour, limit: Duration) -> io::Result<()> {
        let until = Instant::now() + limit;
        self.take_vision();
        loop {
            {
                let rows = self.rows.get_mut().unwrap_or_else(|poisoned| poisoned.into_inner());
                let Some(reader) = rows.as_ref() else { return Err(io::Error::other("The orders layer has no rows at the head")) };
                if self.relearning && reader.frame().is_none() {
                    return Ok(());
                }
                // A whole settle wants the rows of its hour whatever is on the worker.
                reader.ready();
                if let Err(error) = reader.wait_through(hour, until.saturating_duration_since(Instant::now())) {
                    return Err(io::Error::other(format!("{error}: hour {hour} asked")));
                }
            }
            self.exchange(true);
            if self.in_flight == 0 && self.looked == Some(hour) {
                return Ok(());
            }
            if Instant::now() >= until {
                return Err(io::Error::other(format!("The orders layer made no plan at {hour:?} in time: the rows it looked at last stand at {:?}", self.looked)));
            }
        }
    }

    /// Sends the newest rows when they differ from the last sent, and takes what the worker made;
    /// `wait` waits for every build in flight.
    fn exchange(&mut self, wait: bool) {
        if let Some(&(_, ours)) = self.vision.as_ref() {
            let reader = self.rows.get_mut().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref();
            let newest = reader.and_then(|reader| reader.newest().map(|newest| (reader.through(Arc::as_ptr(&newest.frame)), newest)));
            if let Some((stands, Newest { frame, .. })) = newest {
                self.relearning = false;
                self.looked = Some(stands.map_or(frame.hour, |stands| stands.max(frame.hour)));
                let key = (TableKey::of(&*frame, &self.tables), ours, self.show_allied);
                if self.sent.as_ref() == Some(&key) {
                    if self.in_flight == 0 {
                        self.through = self.looked;
                    }
                } else {
                    if self.worker.is_none() {
                        match spawn(Arc::clone(&self.plans)) {
                            Ok(worker) => self.worker = Some(worker),
                            Err(error) => self.refused = Some(format!("The orders layer is refused: its worker cannot start: {error}")),
                        }
                    }
                    if let Some((jobs, _)) = &self.worker
                        && jobs.send(Job { frame, tables: self.tables.clone(), ours, show_allied: self.show_allied }).is_ok()
                    {
                        self.in_flight += 1;
                        self.sent = Some(key);
                    }
                }
            }
        }
        let Some((_, made)) = &self.worker else { return };
        let mut learned = Vec::new();
        while self.in_flight > 0 {
            let next = if wait { made.recv().ok() } else { made.try_recv().ok() };
            let Some(made) = next else { break };
            self.in_flight = self.in_flight.saturating_sub(made.jobs);
            self.marked[2] += made.micros;
            if !made.missed.is_empty() {
                self.marked[1] += 1;
                learned.extend(made.missed);
                continue;
            }
            self.marked[0] += 1;
            self.notes = plan_notes(made.own);
            match made.plan {
                Ok(plan) => {
                    // The flight record's arrows: a plan taken, and one made from rows more than an
                    // hour past the plan before, whose arrows' tips step by more than an hour's
                    // progress (the arrows move only when a plan comes).
                    viewer_foundation::view_record::add("arrows", "installs", 1);
                    if self.plan.as_ref().is_some_and(|(was, _)| made.hour > was + 1) {
                        viewer_foundation::view_record::add("arrows", "steps_over_an_hour", 1);
                    }
                    self.plan = Some((made.hour, Arc::new(plan)));
                    self.refused = None;
                }
                Err(error) => {
                    self.plan = None;
                    self.refused = Some(format!("The orders layer cannot make the plan: {error}"));
                }
            }
        }
        if self.in_flight == 0 && learned.is_empty() && !self.relearning {
            self.through = self.looked;
        }
        // Every job answered, it has finished with the rows it took.
        if self.in_flight == 0
            && let Some(reader) = self.rows.get_mut().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref()
        {
            reader.ready();
        }
        if !learned.is_empty() {
            self.learned.extend(learned);
            self.declare();
            self.relearning = true;
        }
    }

    fn declare(&mut self) {
        let mut tables = orders::memo_tables(self.show_allied);
        tables.extend(self.learned.iter().copied());
        tables.sort_unstable_by_key(|table| table.index());
        tables.dedup();
        if tables != self.tables {
            self.tables = tables;
            self.sent = None;
            if let Some(reader) = self.rows.get_mut().unwrap_or_else(|poisoned| poisoned.into_inner()).as_ref() {
                reader.declare(&self.tables);
            }
        }
    }

    fn take_vision(&mut self) {
        if let Some((choice, vision)) = self.given.get_mut().unwrap_or_else(|poisoned| poisoned.into_inner()).take() {
            self.vision = Some((choice, vision.ours()));
        }
    }
}

impl Layer for Orders {
    fn name(&self) -> &'static str {
        "orders"
    }

    fn placed(&mut self, _view: &crate::layers::View) -> Option<u64> {
        Some(0)
    }

    fn inputs(&mut self, view: &View, inputs: &mut Inputs<'_>) -> io::Result<()> {
        if self.window {
            inputs.after("vision");
        }
        // Its keys only: the hour the map's steps reached, and a reset to compose from.
        inputs.table(TableId::ArmyOrders, true)?;
        if view.graphics.show_allied_plans != self.show_allied {
            self.show_allied = view.graphics.show_allied_plans;
            self.declare();
        }
        Ok(())
    }

    fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
        // The plan held is the old picture's: until one is made from rows after the reset, none.
        self.through = None;
        Ok(())
    }

    fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
        self.head = Some(step.last());
    }

    fn settle_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>, budget: &mut Budget) -> io::Result<()> {
        // Only whose lines or plan are drawn: the viewing country, taken when it changes.
        if self.window
            && let Some(vision) = upstream.get::<crate::layers::vision::Vision>()
            && let Some(ours) = vision.ours()
            && self.vision.as_ref().is_none_or(|(choice, held)| *held != ours || choice != vision.choice())
        {
            self.vision = Some((vision.choice().clone(), ours));
        }
        self.settle(mirrors, budget)
    }

    fn settle(&mut self, mirrors: &Mirrors, budget: &mut Budget) -> io::Result<()> {
        if let Some(head) = mirrors.get(TableId::ArmyOrders).and_then(Mirror::head) {
            self.head = Some(head);
        }
        self.take_vision();
        if self.vision.is_none() {
            self.refused = Some("The orders layer is refused: it is given no vision of the viewing country yet".into());
            return Ok(());
        }
        match self.head {
            // A caller that drives the head settles whole: the plan of the rows at its hour.
            Some(head) if budget.unlimited_now() => self.settle_at(head, Duration::from_secs(60)),
            _ => {
                self.exchange(false);
                Ok(())
            }
        }
    }

    fn upload(&mut self, _gpu: &mut Gpu<'_>, _budget: &mut Budget) -> io::Result<()> {
        Ok(())
    }

    fn carried(&self) -> Carried {
        Carried::default()
    }

    /// While a remake is on the worker (or a table a build read is joining its rows), the hour of
    /// the newest rows the plan it shows stands for; nothing before its first plan. Nothing on the
    /// worker, the plan is of the newest rows: current once they stand at the hour applied, until
    /// then their hour, made on the rows' cadence.
    fn published(&self) -> Published {
        if self.in_flight == 0 && !self.relearning {
            return match (self.through, self.head) {
                (Some(through), Some(head)) if through < head => Published::Cadence(through),
                _ => Published::Current,
            };
        }
        self.through.map_or(Published::Nothing, Published::At)
    }

    fn held(&self) -> usize {
        self.plan.as_ref().map_or(0, |(_, plan)| std::mem::size_of_val(&plan.vertices[..]) + std::mem::size_of_val(&plan.native_vertices[..]) + std::mem::size_of_val(&plan.draws[..]))
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}
