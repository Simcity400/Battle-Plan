//! The battle plan (`OrderField`) from rows at the head, built whole on the layer's own worker as
//! the rows come, never on a step.
//!
//! A plan is one product, not a sum of orders: each order's geometry takes the overlap counters
//! the orders visited before it left (in the roots' drawing order), and each label is placed
//! against the labels before it. So it is made again whole when a table it reads changed, never
//! per order from columns; its tables include the divisions, which change every step in a war, so
//! in a war a plan is made for nearly every step.
//!
//! The worker owns its reader of the map side's view store, whose rows are made from the side's
//! own mirrors inside the step. It builds the newest frame of rows it has not built, publishes
//! the plan at the hour of those rows, and asks for the next; a build begun is always finished and
//! always published, and only the newest frame is ever taken, so the plan is at most one build
//! behind the rows and the hour it stands at only moves forward. A step's settle takes what is
//! published and waits, within the step's budget, only while the plan it holds stands before the
//! step's hour. So the build runs beside the step's other layers instead of after them, and a step
//! whose budget is already spent is offered the plan of an earlier hour rather than none.
//!
//! Ordinary plans read only [`orders::memo_tables`], as today; invasions, active orders and
//! labelled paradrops read the whole hour (their triggers may read any table), so the rows they
//! read are learned as they read them and the plan before them stands until those rows arrive.

#[cfg(test)]
mod tests;

use super::{Carried, Inputs, Layer, Published, Settling, Upstream, View};
use crate::playhead_host::{
    mirror::{Mirror, Mirrors},
    timeline::{self, Mark},
    views::{self, Need},
};
use playhead::Step;
use replay_data::{Hour, rows::TableId};
use replay_frame::frame::{DecodedFrame, discovering};
use std::{
    fmt::Debug,
    io,
    sync::{Arc, Condvar, Mutex, MutexGuard},
    thread::JoinHandle,
    time::{Duration, Instant},
};
use viewer_counter_products::map::orders;
use viewer_foundation::map::{
    memo::{TableKey, project},
    name_draw::{Placements, WorldName},
    perspective::{Choice, Vision},
    ribbon_content::OrderField,
};

/// How long a settle that waits however long the plan takes gives it.
const WHOLE: Duration = Duration::from_secs(60);

/// How long the worker waits for the rows to move before looking at what the layer asks of it (a
/// new viewing country, a new picture): half a frame of a 60 Hz display, since what it looks for
/// there costs far more than that anyway.
const LOOK: Duration = Duration::from_millis(8);

/// What builds a plan from a frame of rows: the recording's content in the viewer.
pub trait Plans: Send + Sync {
    fn build(&self, frame: &DecodedFrame, ours: u32, show_allied: bool) -> io::Result<OrderField>;
}

impl Plans for crate::document::Content {
    /// The memo tables alone unless the hour needs the whole frame, as today's plan.
    fn build(&self, frame: &DecodedFrame, ours: u32, show_allied: bool) -> io::Result<OrderField> {
        // Invasions' triggers read the recording's executable tables.
        engine_executable::executable_tables::scope(self.tables, || {
            let at = replay_frame::cpu::scope("plan.links", Some("plan"));
            let links = crate::document::links_of(frame)?;
            drop(at);
            let at = replay_frame::cpu::scope("plan.requires", Some("plan"));
            let whole = orders::requires_full_frame(frame, &links, ours, show_allied, self.icons.fonts.arrow().is_some())?;
            drop(at);
            if whole {
                self.battle_plan(frame, &links, ours, show_allied)
            } else {
                let at = replay_frame::cpu::scope("plan.project", Some("plan"));
                let only = project(frame, &orders::memo_tables(show_allied));
                drop(at);
                self.battle_plan(&only, &links, ours, show_allied)
            }
        })
    }
}

/// Where two plans' labels part: the first label by key and its first differing field, with both
/// values. In order, since each draw names its label by index; the upload generation aside, as
/// every plan made takes a new one.
pub fn differ_labels(today: &Placements, layer: &Placements) -> Option<String> {
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

/// The newest plan the worker made, at the hour of the rows it was made from.
#[derive(Clone)]
struct Made {
    /// The picture it was made for ([`Held::generation`]).
    generation: u64,
    /// Plans published up to and including it, so a settle takes only one it does not hold.
    stamp: u64,
    /// The hour of the rows it was made from.
    hour: Hour,
    /// The viewing country's slot it was made for.
    ours: u32,
    /// The newest hour those rows stand at: a step that changed no table of theirs moves it with
    /// no build at all ([`RowReader::through`]).
    stands: Hour,
    /// The frame it was made from while that frame is the reader's newest, which is while `stands`
    /// can still move; let go after.
    frame: Option<Arc<DecodedFrame>>,
    plan: Result<Arc<OrderField>, String>,
    /// The battle plan's limitation notes for those rows.
    notes: Vec<String>,
}

/// What the layer and its worker share.
#[derive(Default)]
struct Held {
    /// The viewing country's slot the plan is made for and the allied-plan switch; none until the
    /// layer is given a vision.
    asked: Option<(u32, bool)>,
    /// The picture plans are made for, bumped by every compose: a plan of the picture before is
    /// still drawn, and never counted as this picture's.
    generation: u64,
    made: Option<Made>,
    published: u64,
    /// Why the rows stopped coming or the worker never started, by name: the layer is refused with it.
    stopped: Option<String>,
    /// Plans made, those a table learned made again, and the microseconds they took.
    marked: [u64; 3],
    stop: bool,
}

struct Shared {
    held: Mutex<Held>,
    /// A plan published, or the rows stopped.
    done: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Waits for the worker to publish, until `until`.
    fn wait(&self, until: Instant) {
        let held = self.lock();
        let _ = self.done.wait_timeout(held, until.saturating_duration_since(Instant::now()));
    }
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

/// The tables the readers read: the memo tables of the switch asked and those a build learned.
fn declare(reader: &views::Reader, declared: &mut Vec<TableId>, learned: &[TableId], show_allied: bool) {
    let mut tables = orders::memo_tables(show_allied);
    tables.extend(learned.iter().copied());
    tables.sort_unstable_by_key(|table| table.index());
    tables.dedup();
    if tables != *declared {
        *declared = tables;
        reader.declare_only(declared);
    }
}

/// The worker: every frame of rows it has not built, built whole and published.
fn work(shared: &Shared, plans: &dyn Plans, reader: &views::Reader, maker: timeline::Maker) {
    let mut declared = orders::memo_tables(false);
    let mut learned: Vec<TableId> = Vec::new();
    // What the plan published was made from: the identities of the tables it reads and the ask.
    let mut built: Option<(TableKey, (u32, bool))> = None;
    // The frame it last built or stood the plan at, by the reader's count of frames published.
    let mut handled: Option<(u64, (u32, bool))> = None;
    let mut generation = 0;
    loop {
        let (asked, want) = {
            let held = shared.lock();
            if held.stop {
                return;
            }
            (held.asked, held.generation)
        };
        // A new picture: the plan of the one before is not its plan, however new the rows it holds.
        if want != generation {
            (generation, built, handled) = (want, None, None);
        }
        let Some(asked) = asked else {
            reader.wait_newer(u64::MAX, LOOK);
            continue;
        };
        declare(reader, &mut declared, &learned, asked.1);
        let seen = handled.map_or(0, |(published, _)| published);
        let Some((newest, published)) = reader.newest() else {
            reader.wait_newer(seen, LOOK);
            continue;
        };
        // The frame it has answered already, built or stood: nothing to look at until a newer one.
        if handled == Some((published, asked)) {
            reader.wait_newer(published, LOOK);
            continue;
        }
        // Rows of the seek before are not this picture's: the step after its compose makes the
        // rows of the hour sought.
        let Some((through, seek)) = reader.through_seek(Arc::as_ptr(&newest)) else {
            continue;
        };
        if seek != reader.seek() {
            reader.wait_newer(published, LOOK);
            continue;
        }
        handled = Some((published, asked));
        timeline::mark(maker, Mark::Rows);
        let stands = through.max(newest.hour);
        // The same tables in the same allocations: the plan built from them stands at these rows'
        // hour with no build at all, as the hour the plan is of is their last change.
        let key = (TableKey::of(&*newest, &declared), asked);
        if built.as_ref() == Some(&key) {
            let mut held = shared.lock();
            let stamp = held.published + 1;
            let moved = held.generation == generation
                && held.made.as_mut().is_some_and(|made| {
                    if made.generation != generation {
                        return false;
                    }
                    (made.stands, made.frame, made.stamp) = (made.stands.max(stands), Some(Arc::clone(&newest)), stamp);
                    true
                });
            if moved {
                held.published = stamp;
            }
            drop(held);
            shared.done.notify_all();
            continue;
        }
        let (frame, (ours, show_allied)) = (newest, asked);
        timeline::mark(maker, Mark::BuildBegan);
        let started = Instant::now();
        let ((plan, own), read) = discovering(|| {
            let at = replay_frame::cpu::scope("plan.own_count", Some("plan"));
            let own = crate::document::links_of(&frame).and_then(|links| orders::own_order_count(&frame, &links, ours));
            drop(at);
            (plans.build(&frame, ours, show_allied), own)
        });
        let micros = started.elapsed().as_micros() as u64;
        timeline::mark(maker, Mark::BuildEnded);
        viewer_foundation::view_record::add("arrows", "builds", 1);
        viewer_foundation::view_record::add("arrows", "build_us", micros);
        // Tables the build read that the frame did not hold: the plan is not taken, and the frame
        // after the declaration holds them.
        let missed: Vec<TableId> = read.into_iter().filter(|table| !declared.contains(table)).collect();
        if !missed.is_empty() {
            learned.extend(missed);
            let mut held = shared.lock();
            (held.marked[1], held.marked[2]) = (held.marked[1] + 1, held.marked[2] + micros);
            (built, handled) = (None, None);
            continue;
        }
        let mut held = shared.lock();
        held.marked[2] += micros;
        if held.stop {
            return;
        }
        // A compose while it built: the plan is the picture before's and is not published as this one's.
        if held.generation != generation {
            (built, handled) = (None, None);
            continue;
        }
        held.marked[0] += 1;
        let stamp = held.published + 1;
        held.published = stamp;
        built = Some(key);
        held.made = Some(Made {
            generation,
            stamp,
            hour: frame.hour,
            ours,
            stands,
            frame: Some(frame),
            plan: plan.map(Arc::new).map_err(|error| format!("The orders layer cannot make the plan: {error}")),
            notes: plan_notes(own.map_err(|error| error.to_string())),
        });
        drop(held);
        shared.done.notify_all();
    }
}

pub struct Orders {
    shared: Arc<Shared>,
    plans: Arc<dyn Plans>,
    worker: Mutex<Option<JoinHandle<()>>>,
    given: Mutex<Option<(Choice, Arc<Vision>)>>,
    /// The perspective and the viewing country's slot the plan is made for: all it reads of a
    /// vision.
    vision: Option<(Choice, u32)>,
    /// The side's rows the worker builds from; the layer asks it the hour they stand at.
    rows: Mutex<Option<Arc<views::Reader>>>,
    maker: timeline::Maker,
    /// Registered in the window: its vision is read from the vision layer before it.
    window: bool,
    show_allied: bool,
    /// The last hour the map's steps reached.
    head: Option<Hour>,
    /// The newest plan taken from the worker: drawn whatever picture it was made for, counted as
    /// this one's only while its generation is this one's.
    made: Option<Made>,
    /// The picture it asks plans for, bumped by every compose.
    generation: u64,
    refused: Option<String>,
    marked: [u64; 3],
}

impl Orders {
    pub fn new(plans: Arc<dyn Plans>) -> Self {
        Self {
            shared: Arc::new(Shared { held: Mutex::new(Held::default()), done: Condvar::new() }),
            plans,
            worker: Mutex::new(None),
            given: Mutex::new(None),
            vision: None,
            rows: Mutex::new(None),
            maker: timeline::maker("battle_plan"),
            window: false,
            show_allied: false,
            head: None,
            made: None,
            generation: 0,
            refused: None,
            marked: [0; 3],
        }
    }

    /// The side's view store it builds from, and the worker that builds from it. Given once, as
    /// the side is made.
    pub fn set_views(&self, store: &views::Store) -> io::Result<()> {
        let tables = orders::memo_tables(false);
        let reader = Arc::new(store.reader("orders", &tables.iter().copied().map(Need::every).collect::<Vec<Need>>()));
        *self.rows.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(Arc::clone(&reader));
        let (shared, plans, maker) = (Arc::clone(&self.shared), Arc::clone(&self.plans), self.maker);
        let worker = std::thread::Builder::new().name("battle plan".into()).spawn(move || {
            // A step's settle takes its product: at the head's priority, never below normal.
            viewer_foundation::worker_priority::enter(viewer_foundation::worker_priority::Class::Kept);
            work(&shared, &*plans, &reader, maker);
        });
        match worker {
            Ok(worker) => *self.worker.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(worker),
            Err(error) => self.shared.lock().stopped = Some(format!("The orders layer is refused: its worker cannot start: {error}")),
        }
        Ok(())
    }

    /// Registered in the window: its vision is read from the vision layer before it.
    pub fn in_window(mut self) -> Self {
        self.window = true;
        self
    }

    /// What the viewer sees at the head: whose plan is drawn.
    pub fn set_vision(&self, choice: Choice, vision: Arc<Vision>) {
        *self.given.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((choice, vision));
    }

    pub fn perspective(&self) -> Option<&Choice> {
        self.vision.as_ref().map(|(choice, _)| choice)
    }

    pub fn show_allied(&self) -> bool {
        self.show_allied
    }

    /// The newest plan and the hour of the rows it was made from.
    pub fn plan(&self) -> Option<(Hour, &Arc<OrderField>)> {
        self.shown().and_then(|made| made.plan.as_ref().ok().map(|plan| (made.hour, plan)))
    }

    pub fn refused(&self) -> Option<&str> {
        if let Some(why) = self.shown().and_then(|made| made.plan.as_ref().err()) {
            return Some(why);
        }
        self.refused.as_deref()
    }

    /// The hour the plan it shows stands for: none until one is made for the picture being drawn.
    /// A plan's own hour is its tables' last change, so it stands at every later hour no step
    /// changed a table it reads by.
    pub fn made_for(&self) -> Option<Hour> {
        self.shown().filter(|made| made.generation == self.generation).map(|made| made.stands)
    }

    /// The plan's refusals and missing inputs, worded as today's; loading by name until a plan is made.
    pub fn warnings(&self) -> Vec<String> {
        match (self.plan(), self.refused()) {
            (Some((_, plan)), _) => plan.report.warnings(),
            (None, Some(why)) => vec![format!("Battle plan: not drawn: {why}")],
            (None, None) => vec!["Battle plan: loading at the playback head".to_owned()],
        }
    }

    /// The battle plan's limitation notes in today's words, for the rows its plan was made from:
    /// stated only while the viewing country has orders.
    pub fn notes(&self) -> &[String] {
        self.shown().map_or(&[], |made| &made.notes)
    }

    pub fn marked(&self) -> [u64; 3] {
        self.marked
    }

    /// The plan of the rows at `hour`: the newest plan published is taken, and a step waits for
    /// nothing, keeping the plan it has at that plan's own hour and joining the newer one at the
    /// step after. A settle that waits ([`Settling::waits`]: a whole settle, or this plan at the
    /// bound) waits for the plan of `hour` itself, and a whole settle refuses by name.
    fn settle_at(&mut self, hour: Hour, settling: Settling) -> io::Result<()> {
        if self.rows.get_mut().unwrap_or_else(|poisoned| poisoned.into_inner()).is_none() {
            return Err(io::Error::other("The orders layer has no rows at the head"));
        }
        let until = Instant::now() + settling.waiting(WHOLE);
        self.take_vision();
        let began = Instant::now();
        let mut waited = false;
        loop {
            self.take();
            if self.refused.is_some() {
                if settling.strict() {
                    return Err(io::Error::other(format!("{}: hour {hour} asked; {}", self.refused.as_deref().unwrap_or_default(), self.maker_state())));
                }
                return Ok(());
            }
            // Rows past the hour answer a step (its plan then stands at a newer hour than the
            // layers, as it did before they were joined); a whole settle wants the hour's own.
            if self.made_for().is_some_and(|stands| stands == hour || (stands > hour && !settling.strict())) {
                self.mark_join(began, waited, true);
                timeline::mark(self.maker, Mark::Joined);
                return Ok(());
            }
            if Instant::now() >= until {
                if settling.strict() {
                    return Err(io::Error::other(format!("The orders layer made no plan at {hour:?} in time: the plan it holds stands at {:?}; {}", self.made_for(), self.maker_state())));
                }
                self.mark_join(began, waited, false);
                timeline::mark(self.maker, Mark::Joined);
                return Ok(());
            }
            // The flight record's arrows: whether the step waits for the rows of its hour or for
            // the build of the rows it has.
            if viewer_foundation::view_record::on() {
                let rows = self.rows.get_mut().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
                let held = rows.and_then(|rows| rows.newest()).is_some_and(|(frame, _)| frame.hour >= hour);
                viewer_foundation::view_record::add("arrows", if held { "waits_for_the_build" } else { "waits_for_the_rows" }, 1);
            }
            waited = true;
            self.shared.wait(until);
        }
    }

    /// What the worker and its rows hold when a settle has no plan to take: the picture asked for,
    /// what the worker last published, and what its rows at the head stand at.
    fn maker_state(&self) -> String {
        let reader = self.rows.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
        let newest = reader.as_ref().and_then(|reader| reader.newest());
        let standing = reader.as_ref().zip(newest.as_ref()).and_then(|(reader, (frame, _))| reader.through_seek(Arc::as_ptr(frame)));
        let held = self.shared.lock();
        format!(
            "the layer draws picture {} at head {:?} and holds {:?}; the worker published {}, made {:?} for {:?}, stopped {:?}; its rows: newest {:?}, standing through and at seek {:?} of seek {:?}, arriving {:?}",
            self.generation,
            self.head,
            self.made.as_ref().map(|made| (made.generation, made.hour, made.stands, made.plan.is_ok())),
            held.published,
            held.made.as_ref().map(|made| (made.generation, made.hour, made.stands)),
            held.asked,
            held.stopped,
            newest.as_ref().map(|(frame, published)| (*published, frame.hour)),
            standing,
            reader.as_ref().map(|reader| reader.seek()),
            reader.as_ref().map(|reader| reader.arriving()),
        )
    }

    /// The flight record's arrows: the joins a step made, those that waited at all, and those the
    /// step's budget ran out in.
    fn mark_join(&self, began: Instant, waited: bool, joined: bool) {
        if !viewer_foundation::view_record::on() {
            return;
        }
        viewer_foundation::view_record::add("arrows", "joins", 1);
        viewer_foundation::view_record::add("arrows", "join_us", began.elapsed().as_micros() as u64);
        viewer_foundation::view_record::add("arrows", if waited { "joins_that_waited" } else { "joins_without_a_wait" }, 1);
        if !joined {
            viewer_foundation::view_record::add("arrows", "joins_out_of_budget", 1);
        }
    }

    /// Takes the newest plan published for the picture being drawn, and moves the hour the plan it
    /// holds stands at to every later hour no step changed a table of its rows.
    fn take(&mut self) {
        {
            let held = self.shared.lock();
            self.marked = held.marked;
            if let Some(why) = &held.stopped {
                self.refused = Some(why.clone());
            }
            if let Some(made) = held.made.as_ref().filter(|made| made.generation == self.generation)
                && self.made.as_ref().is_none_or(|ours| ours.stamp < made.stamp)
            {
                // The flight record's arrows: a plan taken, and one made from rows more than an
                // hour past the plan before, whose arrows' tips step by more than an hour's
                // progress (the arrows move only when a plan comes).
                if made.plan.is_ok() && self.made.as_ref().is_none_or(|ours| made.hour != ours.hour) {
                    viewer_foundation::view_record::add("arrows", "installs", 1);
                    if self.made.as_ref().is_some_and(|ours| made.hour > ours.hour + 1) {
                        viewer_foundation::view_record::add("arrows", "steps_over_an_hour", 1);
                    }
                }
                self.made = Some(made.clone());
            }
        }
        let reader = self.rows.get_mut().unwrap_or_else(|poisoned| poisoned.into_inner()).clone();
        if let Some(made) = &mut self.made
            && let Some(frame) = &made.frame
        {
            match reader.and_then(|reader| reader.through(Arc::as_ptr(frame))) {
                Some(through) => made.stands = made.stands.max(through),
                None => made.frame = None,
            }
        }
    }

    /// The plan taken while it was made for the viewing country the layer reads now: one made for
    /// another country is no plan of this one's, and none is drawn until its own is made.
    fn shown(&self) -> Option<&Made> {
        self.made.as_ref().filter(|made| self.vision.as_ref().is_some_and(|&(_, ours)| ours == made.ours))
    }

    /// What the worker makes the plan for: the viewing country's slot and the allied-plan switch.
    fn take_vision(&mut self) {
        if let Some((choice, vision)) = self.given.get_mut().unwrap_or_else(|poisoned| poisoned.into_inner()).take() {
            self.vision = Some((choice, vision.ours()));
        }
        if let Some(&(_, ours)) = self.vision.as_ref() {
            let mut held = self.shared.lock();
            let asked = Some((ours, self.show_allied));
            if held.asked != asked {
                held.asked = asked;
            }
        }
    }
}

impl Drop for Orders {
    fn drop(&mut self) {
        self.shared.lock().stop = true;
        self.shared.done.notify_all();
        if let Some(worker) = self.worker.get_mut().unwrap_or_else(|poisoned| poisoned.into_inner()).take() {
            let _ = worker.join();
        }
    }
}

impl Layer for Orders {
    fn name(&self) -> &'static str {
        "orders"
    }

    /// The side's view store: its worker builds the plan from the rows the step makes of the
    /// side's own mirrors.
    fn views(&mut self, store: &views::Store) -> io::Result<()> {
        self.set_views(store)
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
        self.show_allied = view.graphics.show_allied_plans;
        Ok(())
    }

    fn compose(&mut self, _mirrors: &Mirrors) -> io::Result<()> {
        // The plan held is the old picture's: it is drawn until one is made from rows after the
        // reset, and counted as this picture's only then.
        self.generation += 1;
        self.shared.lock().generation = self.generation;
        Ok(())
    }

    fn apply(&mut self, step: &Step, _mirrors: &Mirrors) {
        self.head = Some(step.last());
    }

    /// What the plan is made for: the step's hour and the viewing country, which is all the worker
    /// needs to build the plan of the rows it is given next.
    fn ask(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>) -> io::Result<bool> {
        // Only whose lines or plan are drawn: the viewing country, taken when it changes; none while
        // another country chosen has no vision yet (being made or refused), so no plan of the
        // country before stands. A seek's vision made again keeps the plan before until the next.
        if self.window
            && let Some(vision) = upstream.get::<crate::layers::vision::Vision>()
        {
            match vision.ours() {
                Some(ours) if self.vision.as_ref().is_none_or(|(choice, held)| *held != ours || choice != vision.choice()) => {
                    self.vision = Some((vision.choice().clone(), ours));
                }
                None if self.vision.as_ref().is_some_and(|(choice, _)| choice != vision.choice()) => self.vision = None,
                _ => {}
            }
        }
        if let Some(head) = mirrors.get(TableId::ArmyOrders).and_then(Mirror::head) {
            self.head = Some(head);
        }
        self.take_vision();
        Ok(true)
    }

    fn join(&mut self, _mirrors: &Mirrors, _upstream: &Upstream<'_>, settling: Settling) -> io::Result<()> {
        if self.vision.is_none() {
            self.refused = Some("The orders layer is refused: it is given no vision of the viewing country yet".into());
            return Ok(());
        }
        // Taken again from the worker, which refuses it by name while the rows have stopped.
        self.refused = None;
        match self.head {
            // A caller that drives the head settles whole: the plan of the rows at its hour.
            Some(head) => self.settle_at(head, settling),
            None => {
                self.take();
                Ok(())
            }
        }
    }

    fn settle_with(&mut self, mirrors: &Mirrors, upstream: &Upstream<'_>, settling: Settling) -> io::Result<()> {
        self.ask(mirrors, upstream)?;
        self.join(mirrors, upstream, settling)
    }

    fn settle(&mut self, mirrors: &Mirrors, settling: Settling) -> io::Result<()> {
        self.settle_with(mirrors, &Upstream::new(&[]), settling)
    }

    fn carried(&self) -> Carried {
        Carried::default()
    }

    /// The hour the plan it shows stands at: the applied hour once a plan standing there is
    /// published ([`Published::Current`]), until then the hour the plan it holds stands at, made on
    /// the rows' cadence, and nothing before the first plan of the picture being drawn.
    fn published(&self) -> Published {
        let Some(stands) = self.made_for() else { return Published::Nothing };
        match self.head {
            Some(head) if stands < head => Published::Cadence(stands),
            _ => Published::Current,
        }
    }

    fn held(&self) -> usize {
        self.plan().map_or(0, |(_, plan)| std::mem::size_of_val(&plan.vertices[..]) + std::mem::size_of_val(&plan.native_vertices[..]) + std::mem::size_of_val(&plan.draws[..]))
    }

    fn any(&self) -> &dyn std::any::Any {
        self
    }
}
