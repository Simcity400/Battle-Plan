//! Each admitted land stack's mesh, chosen by the units layer on a worker of its own over the rows
//! the map side makes from its own mirrors inside its step ([`views::Views::telling`]), never a
//! stepped copy of its own.
//!
//! The worker owns its reader of the side's view store, whose rows stand at the hour the side
//! applied. The layer hands it, at the step's apply, the viewer and the stacks whose figure or
//! selection the settle changed: it chooses over the newest frame it has not chosen from,
//! publishes the meshes that changed at that frame's hour, and asks for the next. So the session
//! runs beside the step's other makers instead of after them, and the layer's join takes what is
//! published and waits, within the step's budget, only while the meshes it holds stand before the
//! step's hour.
//!
//! A session runs only when stacks were handed or a table it reads was made anew since the rows the
//! last one read; the Clock is one, so each hour a session decides the graphics pools' limits
//! again, which may compare the date. The store tells which rows of each table it made anew,
//! so the session finds the kept answers they reach without comparing tables ([`Told`]): it chooses
//! again the stacks handed and those whose kept answers fell, every other keeping its mesh unasked.
//! A whole settle waits for the rows at its hour and for every stack handed to be chosen. A table
//! the selection's scripts read beyond [`TABLES`] is learned and declared: the stacks keep their
//! meshes until its rows arrive, never a guess.

use super::models::Select;
use crate::{
    layers::{
        Settling,
        rebuilt::{absent, refusal},
    },
    playhead_host::{
        Entry,
        timeline::{self, Mark},
        views::{self, HeldKeys, Need},
    },
};
use replay_data::{Hour, rows::TableId};
use replay_frame::frame::{DecodedFrame, SharedView, TableView, discovering};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    thread::JoinHandle,
    time::{Duration, Instant},
};
use viewer_world::map::unit_models::{KeptChoices, Made, SelectionKey, Told};

/// The tables the choice of a land stack's model is known to read: the oldest member's template,
/// equipment and status, the variants, the countries' appearance, politics, factions and scripted
/// values the graphics pools evaluate, the province, state and weather of the camouflage, and the
/// Clock, whose hour the pools' limits may compare (rows at every step, so the session decides
/// them again each hour). A table the content's scripts read beyond them is learned.
pub const TABLES: &[TableId] = &[
    TableId::Clock,
    TableId::DivisionTemplateIcons,
    TableId::DivisionModelInputs,
    TableId::DivisionStatus,
    TableId::VariantDefinitions,
    TableId::VariantUpgrades,
    TableId::AvailableEquipmentVariants,
    TableId::Countries,
    TableId::CountryLinks,
    TableId::CountryAppearance,
    TableId::CountryLocalSubunits,
    TableId::Politics,
    TableId::PoliticalParties,
    TableId::Factions,
    TableId::FactionMembers,
    TableId::LexicalTokens,
    TableId::ScriptVariables,
    TableId::SimulationFeatures,
    TableId::Provinces,
    TableId::States,
    TableId::WeatherProvinces,
    TableId::WeatherRegions,
];

/// The equipment tables, read by key ([`views::Reader::declare_keyed`]): the rows of the divisions
/// of the stacks held alone, a few hundred of some sixty thousand.
pub const KEYED: [TableId; 2] = [TableId::DivisionEquipment, TableId::NewDivisionEquipment];

/// The weather columns the camouflage reads, of sixteen: a province's current and permanent snow.
/// The rest hold its temperature pass, its modifier accumulator and its day/night record, whose
/// rows cost more to make than every other table of [`TABLES`] together.
const SNOW: u64 = replay_data::rows::mask::<replay_data::rows::WeatherProvinceRow>(&["snow", "permanent_snow"]);

/// `tables` with the columns read of each: every column, but the weather's snow alone. A table
/// `learned` is read whole, which widens the weather again if the camouflage ever reads more of it.
fn needs(tables: &[TableId], learned: &[TableId]) -> Vec<Need> {
    let named = |table: TableId| if table == TableId::WeatherProvinces { Need::columns(table, SNOW) } else { Need::every(table) };
    tables.iter().copied().map(named).chain(learned.iter().copied().map(Need::every)).collect()
}

/// How long a whole settle waits for the rows at its hour, and for the meshes chosen from them.
pub const WHOLE: Duration = Duration::from_secs(10);

/// How long the worker waits for the rows to move before looking at the stacks the layer hands it:
/// a quarter of a frame of a 120 Hz display. The rows of every step move the Clock the selection
/// reads, so while the map plays the rows are what wakes it and this bounds only a camera move or a
/// seek that hands stacks on with no step behind them.
const LOOK: Duration = Duration::from_millis(2);

/// Where a stack stands and what its model is selected by, held by its oldest seen member.
pub type Asked = (u32, SelectionKey);

/// A stack's mesh; none when it has no model (its country's operands unavailable, its template not
/// recorded this hour); or why it is withheld, by name.
pub type Mesh = Result<Option<usize>, String>;

/// What one session chose: each stack chosen again with its mesh, and what the session made rather
/// than kept.
pub struct Chosen {
    pub meshes: Vec<(u64, Mesh)>,
    pub made: Made,
}

/// A stack the layer hands on: its figure asks a mesh, or no longer does.
pub enum Ask {
    Stack(u64, Asked),
    Gone(u64),
}

/// One session's answer: the meshes that changed, at the hour of the rows it read, and whether
/// every mesh held is dropped before them.
struct Product {
    clear: bool,
    meshes: Vec<(u64, Mesh)>,
}

/// What the frame took from the worker since last asked: sessions started, products, stacks chosen
/// again, selections made new and looked up by their selection key, camouflages and country
/// contexts made, and the worker's time, by part of a session too; the stacks that entered and left
/// the asked set. And as they stand when asked: the stacks waiting for their rows, the keys
/// declared and the rows the newest frame holds of them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub builds: u64,
    pub products: u64,
    pub chosen: u64,
    pub selections: u64,
    pub looked_up: u64,
    pub camouflages: u64,
    pub contexts: u64,
    pub busy_ns: u64,
    pub parts: Vec<(&'static str, u64)>,
    pub entering: u64,
    pub leaving: u64,
    pub waiting: u64,
    pub keys: u64,
    pub keyed_rows: u64,
}

/// What the layer and its worker share.
#[derive(Default)]
struct Held {
    /// The viewing country the meshes are chosen for; none until the layer has one.
    ours: Option<u32>,
    /// The stacks handed since the worker last looked.
    asks: Vec<Ask>,
    /// Every stack held is forgotten before the next session.
    clear: bool,
    /// Bumped by each clear: what a session begun before it answers is dropped.
    epoch: u64,
    /// What the worker chose and the layer has not taken, oldest first.
    made: Vec<Product>,
    /// The hour of the rows the meshes held were chosen from, moved on to every later hour no step
    /// changed a table of those rows by.
    hour: Option<Hour>,
    /// The hour the newest rows the worker has answered stand at: it owes nothing up to there.
    answered: Option<Hour>,
    refused: Option<String>,
    /// The stacks handed are not all chosen: a session running, a pass owed, or a stack waiting for
    /// the rows of its equipment.
    owed: bool,
    /// The tables it reads, the learned ones included, those the recording holds nothing of left out.
    tables: Vec<TableId>,
    counts: Counts,
    /// As they stand: the stacks waiting for their rows, the keys declared, and the rows the newest
    /// frame holds of them.
    waiting: u64,
    keys: u64,
    keyed_rows: u64,
    stop: bool,
}

struct Shared {
    held: Mutex<Held>,
    /// Meshes published, or the rows stopped.
    done: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Held> {
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait(&self, until: Instant) {
        let held = self.lock();
        let _ = self.done.wait_timeout(held, until.saturating_duration_since(Instant::now()));
    }

    /// The rows the meshes were chosen from stand at `stands`: no session is owed for them.
    fn stood(&self, stands: Hour) {
        let mut held = self.lock();
        if held.hour.is_some() {
            held.hour = held.hour.max(Some(stands));
        }
        held.answered = held.answered.max(Some(stands));
        drop(held);
        self.done.notify_all();
    }
}

pub struct Meshes {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    maker: timeline::Maker,
    /// Each stack's mesh by its oldest member, as the worker last answered, and why a stack has none.
    pub meshes: HashMap<u64, usize>,
    pub withheld: HashMap<u64, String>,
    /// The hour of the rows the meshes are made from.
    pub hour: Option<Hour>,
    pub refused: Option<String>,
}

impl Meshes {
    /// The worker, reading the side's view store for [`TABLES`] and the tables `learned` before.
    pub fn start(select: Arc<dyn Select>, store: &views::Views, learned: &[TableId]) -> Self {
        let entry = Arc::clone(store.entry());
        let mut tables: Vec<TableId> = TABLES.iter().chain(learned).copied().filter(|table| !KEYED.contains(table)).collect();
        tables.sort_unstable_by_key(|table| table.index());
        tables.dedup();
        let every: Vec<TableId> = tables.iter().chain(&KEYED).copied().collect();
        let absent = absent(&entry, &every);
        tables.retain(|table| !absent.contains(table));
        let keyed: Vec<TableId> = KEYED.into_iter().filter(|table| !absent.contains(table)).collect();
        let learned: Vec<TableId> = learned.iter().copied().filter(|table| tables.contains(table)).collect();
        let reader = store.telling("unit_meshes", &needs(&tables, &learned));
        let declared = Arc::new(BTreeSet::new());
        for &table in &keyed {
            reader.declare_keyed(table, Arc::clone(&declared));
        }
        let maker = timeline::maker("unit_meshes");
        let shared = Arc::new(Shared { held: Mutex::new(Held { tables: tables.clone(), clear: true, ..Held::default() }), done: Condvar::new() });
        let run = Run {
            select,
            entry,
            absent: Vec::new(),
            held: HashMap::new(),
            asked: HashSet::new(),
            kept: KeptChoices::default(),
            last: None,
            kept_at: None,
            since: BTreeMap::new(),
            tables,
            learned,
            keyed,
            divisions: BTreeSet::new(),
            declared,
            waiting: Vec::new(),
            read: None,
            clear: false,
            again: false,
        };
        let spawned = std::thread::Builder::new().name("unit meshes".into()).spawn({
            let shared = Arc::clone(&shared);
            move || {
                // The step's settle joins its product: at the head's priority, never below normal.
                viewer_foundation::worker_priority::enter(viewer_foundation::worker_priority::Class::Kept);
                run.started(&absent).work(&shared, &reader, maker);
            }
        });
        let (worker, refused) = match spawned {
            Ok(worker) => (Some(worker), None),
            Err(error) => (None, Some(format!("The unit meshes' worker cannot start: {error}"))),
        };
        Self { shared, worker, maker, meshes: HashMap::new(), withheld: HashMap::new(), hour: None, refused }
    }

    /// Its place in the step's timeline.
    pub fn maker(&self) -> timeline::Maker {
        self.maker
    }

    /// Every stack handed before is forgotten: the next session is handed every stack anew, and
    /// what a session begun before this answers is dropped.
    pub fn clear(&mut self) {
        let mut held = self.shared.lock();
        held.asks.clear();
        (held.clear, held.epoch, held.answered) = (true, held.epoch + 1, None);
    }

    /// The viewer and the stacks the settle touched handed to the worker, with the step's apply:
    /// it waits for the rows of the step's hour itself and chooses over them while the step's other
    /// layers settle.
    pub fn ask(&mut self, ours: u32, asks: Vec<Ask>) {
        let mut held = self.shared.lock();
        held.ours = Some(ours);
        held.asks.extend(asks);
        drop(held);
        timeline::mark(self.maker, Mark::Asked);
    }

    /// The meshes of the stacks asked joined to the step: what the worker published is taken and
    /// a step waits for nothing, joining what is not ready at the step after. A settle that waits
    /// ([`Settling::waits`]) waits while the rows it has answered stand before `applied`, and, for
    /// a settle that waits for everything, while a stack handed is not chosen. Whether a mesh
    /// changed.
    pub fn join(&mut self, applied: Hour, settling: Settling) -> bool {
        let until = Instant::now() + settling.waiting(WHOLE);
        let strict = settling.strict();
        let mut changed = false;
        loop {
            changed |= self.take();
            if self.shared.lock().answered.is_some_and(|at| at >= applied) && (!strict || self.stand()) {
                // An answer published between the take above and the check is taken too: it was
                // pushed with the hour that satisfied the check.
                return changed | self.take();
            }
            if Instant::now() >= until {
                return changed;
            }
            self.shared.wait(until);
        }
    }

    /// Takes every mesh the worker published; whether one changed.
    fn take(&mut self) -> bool {
        let (made, hour, refused) = {
            let mut held = self.shared.lock();
            (std::mem::take(&mut held.made), held.hour, held.refused.clone())
        };
        let mut changed = false;
        for product in made {
            if product.clear {
                changed |= !self.meshes.is_empty() || !self.withheld.is_empty();
                self.meshes.clear();
                self.withheld.clear();
            }
            for (oldest, mesh) in product.meshes {
                changed |= match mesh {
                    Ok(Some(mesh)) => {
                        self.withheld.remove(&oldest);
                        self.meshes.insert(oldest, mesh) != Some(mesh)
                    }
                    Ok(None) => {
                        self.withheld.remove(&oldest);
                        self.meshes.remove(&oldest).is_some()
                    }
                    Err(why) => {
                        self.withheld.insert(oldest, why);
                        self.meshes.remove(&oldest).is_some()
                    }
                };
            }
        }
        (self.hour, self.refused) = (hour, refused);
        changed
    }

    /// The tables it reads, the learned ones included.
    pub fn tables(&self) -> Vec<TableId> {
        self.shared.lock().tables.clone()
    }

    /// Whether it has made no meshes yet: its first rows or its first session are on their way.
    pub fn waiting(&self) -> bool {
        self.hour.is_none() && self.refused.is_none()
    }

    /// Whether every stack handed has been chosen by a session that has answered: nothing the layer
    /// handed is still queued, nothing is owed, and no session is running. A stack still owed has
    /// no mesh, so its figure is not in the field: the hour the meshes stand at is the hour of the
    /// rows they read, which says nothing about a stack handed since. A refused worker owes
    /// nothing. What a saved frame waits for.
    pub fn stand(&self) -> bool {
        let held = self.shared.lock();
        held.refused.is_some() || (!held.owed && !held.clear && held.asks.iter().all(|ask| matches!(ask, Ask::Gone(_))))
    }

    /// What it took since the last call, with the stacks waiting, the keys declared and the rows
    /// the newest frame holds of them as the worker last saw them.
    pub fn take_counts(&self) -> Counts {
        let mut held = self.shared.lock();
        let (waiting, keys, keyed_rows) = (held.waiting, held.keys, held.keyed_rows);
        let mut counts = std::mem::take(&mut held.counts);
        (counts.waiting, counts.keys, counts.keyed_rows) = (waiting, keys, keyed_rows);
        counts
    }

    /// Its levels for the flight record ([`viewer_foundation::view_record`]): the stacks waiting for
    /// their rows, the keys declared, and the rows its newest frame holds of them.
    pub fn record_view(&self) {
        use viewer_foundation::view_record::set;
        let held = self.shared.lock();
        set("units", "choices_waiting", held.waiting);
        set("units", "keys_declared", held.keys);
        set("units", "rows_by_key", held.keyed_rows);
        drop(held);
        // A stack asked draws its figure once it holds a mesh; one withheld draws none, by name.
        set("units", "meshes_held", self.meshes.len() as u64);
        set("units", "meshes_withheld", self.withheld.len() as u64);
    }

    /// What it asks, holds and waits for, for a test's message.
    #[cfg(any(test, feature = "test-support"))]
    pub fn state(&self) -> String {
        let held = self.shared.lock();
        format!(
            "keys declared {}; asks queued {}, waiting for their rows {}, owed {}, clear {}, hour {:?}, answered {:?}, refused {:?}",
            held.keys,
            held.asks.len(),
            held.waiting,
            held.owed,
            held.clear,
            held.hour,
            held.answered,
            held.refused
        )
    }

    pub fn bytes(&self) -> usize {
        self.meshes.capacity() * (8 + 8 + 8) + self.withheld.capacity() * (8 + 24 + 8) + self.withheld.values().map(String::capacity).sum::<usize>()
    }
}

impl Drop for Meshes {
    fn drop(&mut self) {
        self.shared.lock().stop = true;
        self.shared.done.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// The worker: the rows it reads, the stacks it holds and the answers it keeps between sessions.
struct Run {
    select: Arc<dyn Select>,
    entry: Arc<Entry>,
    /// Each table the recording holds nothing of, read as one view that refuses it by name.
    absent: Vec<(TableId, SharedView)>,
    held: HashMap<u64, Asked>,
    /// Stacks handed and not chosen yet.
    asked: HashSet<u64>,
    kept: KeptChoices,
    /// The rows of the session before, and those its kept answers were kept at with the rows of
    /// each table made anew since (none: made whole).
    last: Option<Arc<DecodedFrame>>,
    kept_at: Option<DecodedFrame>,
    since: BTreeMap<TableId, (SharedView, Option<BTreeMap<u128, u64>>)>,
    /// The tables it reads, the learned ones included, those the recording holds nothing of left out.
    tables: Vec<TableId>,
    /// Those of them a session was found to read beyond [`TABLES`]: read whole, columns and all.
    learned: Vec<TableId>,
    /// The equipment tables it reads by key (those the recording holds), the oldest divisions of
    /// the stacks handed and not let go, and the set last declared of them.
    keyed: Vec<TableId>,
    divisions: BTreeSet<u64>,
    declared: Arc<BTreeSet<u64>>,
    /// Stacks handed whose equipment rows the frame does not hold yet.
    waiting: Vec<Ask>,
    /// The rows the newest session read and the viewer it chose for.
    read: Option<(Arc<DecodedFrame>, u32)>,
    /// Every stack held is forgotten before the next session.
    clear: bool,
    /// The stacks handed wait for another session: the last read a table it had not declared.
    again: bool,
}

impl Run {
    /// `tables` the recording holds nothing of are read as refused by name.
    fn started(mut self, absent: &[TableId]) -> Self {
        self.refuse(absent);
        self
    }

    fn refuse(&mut self, tables: &[TableId]) {
        let mut every: Vec<TableId> = self.absent.iter().map(|(table, _)| *table).chain(tables.iter().copied()).collect();
        every.sort_unstable_by_key(|table| table.index());
        every.dedup();
        if every.is_empty() {
            return;
        }
        let reason = refusal("the land unit models", &every);
        self.absent = every.into_iter().map(|table| (table, SharedView::from(TableView::Unavailable { reason: reason.clone() }))).collect();
    }

    /// Takes `asks` into the stacks asked: their oldest divisions are the keys.
    fn note(&mut self, shared: &Shared, asks: &[Ask]) {
        let mut counts = (0, 0);
        for ask in asks {
            match ask {
                Ask::Stack(oldest, _) if self.divisions.insert(*oldest) => counts.0 += 1,
                Ask::Gone(oldest) if self.divisions.remove(oldest) => counts.1 += 1,
                _ => {}
            }
        }
        viewer_foundation::view_record::add("units", "entering", counts.0);
        viewer_foundation::view_record::add("units", "leaving", counts.1);
        let mut held = shared.lock();
        (held.counts.entering, held.counts.leaving) = (held.counts.entering + counts.0, held.counts.leaving + counts.1);
    }

    /// The rows of every stack asked are declared by key: those of a stack handed since come in
    /// the reader's next frame.
    fn declare_keys(&mut self, reader: &views::Reader) {
        if *self.declared == self.divisions {
            return;
        }
        self.declared = Arc::new(self.divisions.clone());
        for &table in &self.keyed {
            reader.declare_keyed(table, Arc::clone(&self.declared));
        }
    }

    /// Whether a frame whose tables read by key hold `keys` holds the rows `ask` reads; every stack
    /// asked, when `ask` is none.
    fn holds(&self, keys: &HeldKeys, ask: Option<&Ask>) -> bool {
        self.keyed.iter().all(|&table| {
            let Some(held) = keys.of(table) else { return false };
            match ask {
                Some(Ask::Gone(_)) => true,
                Some(Ask::Stack(oldest, _)) => held.contains(oldest),
                None => self.divisions.is_subset(held),
            }
        }) || ask.is_some_and(|ask| matches!(ask, Ask::Gone(_)))
    }

    /// Whether the stacks handed are not all chosen.
    fn owed(&self) -> bool {
        self.clear || self.again || self.waiting.iter().any(|ask| matches!(ask, Ask::Stack(..)))
    }

    /// What it stands at, for the layer and the flight record.
    fn publish(&self, shared: &Shared, reader: &views::Reader, running: bool) {
        let waiting = self.waiting.iter().filter(|ask| matches!(ask, Ask::Stack(..))).count() as u64;
        let keyed_rows = viewer_foundation::view_record::on().then(|| self.keyed_rows(reader));
        let mut held = shared.lock();
        held.owed = running || self.owed();
        (held.waiting, held.keys) = (waiting, self.declared.len() as u64);
        if let Some(rows) = keyed_rows {
            held.keyed_rows = rows;
        }
    }

    /// The rows the newest frame holds of the tables read by key.
    fn keyed_rows(&self, reader: &views::Reader) -> u64 {
        reader.frame().map_or(0, |frame| {
            self.keyed
                .iter()
                .map(|&table| match frame.tables[table.index()].held() {
                    TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.len() as u64,
                    _ => 0,
                })
                .sum()
        })
    }

    /// Every frame of rows it has not chosen over, chosen and published.
    fn work(mut self, shared: &Shared, reader: &views::Reader, maker: timeline::Maker) {
        loop {
            let (ours, asks, clear, epoch) = {
                let mut held = shared.lock();
                if held.stop {
                    return;
                }
                let asks = std::mem::take(&mut held.asks);
                let clear = std::mem::replace(&mut held.clear, false);
                if clear || asks.iter().any(|ask| matches!(ask, Ask::Stack(..))) {
                    held.owed = true;
                }
                (held.ours, asks, clear, held.epoch)
            };
            if clear {
                self.forget();
            }
            if !asks.is_empty() {
                self.note(shared, &asks);
                // A stack's latest ask stands: one withdrawn after it was asked is not waited for
                // (its rows are no longer declared), and the withdrawal still lets its mesh go.
                for ask in asks {
                    let oldest = match &ask {
                        Ask::Stack(oldest, _) | Ask::Gone(oldest) => *oldest,
                    };
                    if matches!(ask, Ask::Gone(_)) {
                        self.waiting.retain(|held| !matches!(held, Ask::Stack(stack, _) if *stack == oldest));
                    }
                    self.waiting.push(ask);
                }
                self.declare_keys(reader);
            }
            let (Some(ours), Some((newest, published))) = (ours, reader.newest()) else {
                self.publish(shared, reader, false);
                reader.wait_newer(0, LOOK);
                continue;
            };
            let Some((frame, keys)) = reader.frame_keyed() else { continue };
            // The keys are the frame's own: a frame taken between the two reads is looked at again.
            if !Arc::ptr_eq(&newest, &frame) {
                continue;
            }
            // The last session read these rows: they stand at every hour no step since changed a
            // table they hold.
            if !self.again
                && let Some((read, _)) = &self.read
                && let Some(stands) = reader.through(Arc::as_ptr(read))
            {
                shared.stood(stands);
            }
            let made_anew = self.read.as_ref().is_none_or(|(read, _)| {
                self.tables.iter().copied().chain(self.keyed.iter().copied()).any(|table| !SharedView::ptr_eq(&read.tables[table.index()], &frame.tables[table.index()]))
            });
            let sendable = self.waiting.iter().any(|ask| self.holds(&keys, Some(ask)));
            let asked = self.clear || self.again || sendable || self.read.as_ref().is_none_or(|&(_, viewer)| viewer != ours);
            if !made_anew && !asked {
                // Nothing it reads was made anew: the meshes stand for these rows, through the hour
                // no step since changed them by.
                shared.stood(reader.through(Arc::as_ptr(&frame)).unwrap_or(frame.hour).max(frame.hour));
                self.publish(shared, reader, false);
                reader.wait_newer(published, LOOK);
                continue;
            }
            let Some((frame, told, keys)) = reader.take_keyed() else { continue };
            timeline::mark(maker, Mark::Rows);
            // A stack whose rows the frame does not hold yet waits for the frame that does: never
            // chosen as if it had none. The keys are the taken frame's own.
            let (chosen, waiting): (Vec<Ask>, Vec<Ask>) = std::mem::take(&mut self.waiting).into_iter().partition(|ask| self.holds(&keys, Some(ask)));
            self.waiting = waiting;
            let clear = std::mem::take(&mut self.clear);
            self.again = false;
            {
                let mut held = shared.lock();
                (held.counts.builds, held.owed) = (held.counts.builds + 1, true);
            }
            timeline::mark(maker, Mark::BuildBegan);
            let started = Instant::now();
            let answer = self.answer(&frame, told, &keys, ours, clear, chosen);
            timeline::mark(maker, Mark::BuildEnded);
            let busy_ns = started.elapsed().as_nanos() as u64;
            // A table the session read undeclared: declared now, the stacks keeping their meshes
            // until its rows arrive.
            if self.again {
                reader.declare_only_needs(&needs(&self.tables, &self.learned));
                shared.lock().tables.clone_from(&self.tables);
            }
            // A clear while it chose: those meshes are the picture before's and are not taken.
            if shared.lock().epoch != epoch {
                continue;
            }
            self.publish_answer(shared, answer, busy_ns, &frame);
            self.publish(shared, reader, false);
            shared.done.notify_all();
        }
    }

    /// Everything held is forgotten: a reset of the picture.
    fn forget(&mut self) {
        self.held.clear();
        self.asked.clear();
        self.divisions.clear();
        self.waiting.clear();
        self.read = None;
        self.again = false;
        self.clear = true;
    }

    /// One session's answer published to the layer, with what it cost.
    fn publish_answer(&self, shared: &Shared, answer: Answer, busy_ns: u64, frame: &Arc<DecodedFrame>) {
        let Answer { meshes, made, chosen, refused, clear } = answer;
        let mut held = shared.lock();
        let counts = &mut held.counts;
        counts.products += 1;
        counts.chosen += chosen as u64;
        counts.selections += made.selections as u64;
        // Each stack chosen took its selection by its key from the kept answers, or made it new.
        let looked_up = (chosen as u64).saturating_sub(made.selections as u64);
        counts.looked_up += looked_up;
        counts.camouflages += made.camouflages as u64;
        counts.contexts += made.contexts as u64;
        counts.busy_ns += busy_ns;
        for &(name, ns) in &made.parts {
            match counts.parts.iter_mut().find(|(held, _)| *held == name) {
                Some((_, sum)) => *sum += ns,
                None => counts.parts.push((name, ns)),
            }
        }
        viewer_foundation::view_record::add("units", "choices_looked_up", looked_up);
        viewer_foundation::view_record::add("units", "choices_made_new", made.selections as u64);
        held.refused = refused;
        // A session that waits for a learned table's rows answers only the stacks let go.
        if !self.again && held.refused.is_none() {
            held.hour = Some(frame.hour);
        }
        held.answered = held.answered.max(Some(frame.hour));
        held.made.push(Product { clear, meshes });
    }

    /// One session over `frame`: the stacks `chosen` names and those whose kept answers fell.
    fn answer(&mut self, frame: &Arc<DecodedFrame>, told: views::Told, keys: &HeldKeys, ours: u32, clear: bool, chosen: Vec<Ask>) -> Answer {
        let mut answer = Answer { meshes: Vec::new(), made: Session::default(), chosen: 0, refused: None, clear };
        let mut since = Instant::now();
        let mut lap = |parts: &mut Vec<(&'static str, u64)>, name: &'static str| {
            let now = Instant::now();
            parts.push((name, now.duration_since(since).as_nanos() as u64));
            since = now;
        };
        for ask in chosen {
            match ask {
                Ask::Stack(oldest, asked) => {
                    self.held.insert(oldest, asked);
                    self.asked.insert(oldest);
                }
                Ask::Gone(oldest) => {
                    self.held.remove(&oldest);
                    self.asked.remove(&oldest);
                    answer.meshes.push((oldest, Ok(None)));
                }
            }
        }
        let tables: Vec<TableId> = self.tables.iter().copied().chain(self.keyed.iter().copied()).collect();
        // The session reads these rows: a refusal is not tried again over the same ones.
        self.read = Some((Arc::clone(frame), ours));
        self.note_rows(frame, &tables, told);
        lap(&mut answer.made.parts, "asks_and_told");
        let mut rows = DecodedFrame::clone(frame);
        for (table, view) in &self.absent {
            rows.tables[table.index()] = view.clone();
        }
        let told = Told::new(self.since.iter().filter_map(|(&table, (from, rows))| Some((table, from.clone(), rows.as_ref()?.iter().map(|(&key, &fields)| (key, fields)).collect()))).collect());
        lap(&mut answer.made.parts, "rows_and_told");
        let mut session = Vec::new();
        let keyed = keys.of(TableId::DivisionEquipment).or_else(|| keys.of(TableId::NewDivisionEquipment)).map(|keys| &**keys);
        let (made, missed) = discovering(|| self.select.choose_meshes(&rows, ours, (&self.kept, keyed), &self.held, &self.asked, &told, &mut |name, ns| session.push((name, ns))));
        answer.made.parts.extend(session);
        match made {
            Ok(Chosen { meshes, made }) => {
                // The session ran: its answers are kept at these rows.
                (self.kept_at, self.since) = (Some(rows), BTreeMap::new());
                (answer.made.selections, answer.made.camouflages, answer.made.contexts) = (made.selections, made.camouflages, made.contexts);
                let missed: Vec<TableId> = missed.into_iter().filter(|table| !tables.contains(table) && !self.absent.iter().any(|(held, _)| held == table)).collect();
                if !missed.is_empty() {
                    let gone = absent(&self.entry, &missed);
                    self.refuse(&gone);
                    let learned: Vec<TableId> = missed.into_iter().filter(|table| !gone.contains(table)).collect();
                    if !learned.is_empty() {
                        self.learn(&learned);
                    }
                    self.again = true;
                    return answer;
                }
                self.asked.clear();
                answer.chosen = meshes.len();
                answer.meshes.extend(meshes);
            }
            Err(error) => answer.refused = Some(format!("The land unit models cannot be chosen at the head: {error}")),
        }
        answer
    }

    /// `learned` declared from now on: the stacks keep their meshes until its rows arrive.
    fn learn(&mut self, learned: &[TableId]) {
        self.learned.extend(learned.iter().copied());
        self.tables.extend(learned.iter().copied());
        for held in [&mut self.tables, &mut self.learned] {
            held.sort_unstable_by_key(|table| table.index());
            held.dedup();
        }
    }

    /// Adds the rows of `tables` made anew since the session before, `told`, to those made anew
    /// since the rows the kept answers were kept at; a table whose rows differ untold counts as
    /// made whole.
    fn note_rows(&mut self, frame: &Arc<DecodedFrame>, tables: &[TableId], mut told: views::Told) {
        let last = self.last.replace(Arc::clone(frame));
        let Some(kept_at) = &self.kept_at else { return };
        for &table in tables {
            let index = table.index();
            let rows = match told.remove(&table) {
                Some(rows) => rows,
                None if last.as_ref().is_some_and(|last| SharedView::ptr_eq(&last.tables[index], &frame.tables[index])) => continue,
                None => None,
            };
            let (_, held) = self.since.entry(table).or_insert_with(|| (kept_at.tables[index].clone(), Some(BTreeMap::new())));
            match (held, rows) {
                (Some(held), Some(rows)) => {
                    for (key, fields) in rows {
                        *held.entry(key).or_default() |= fields;
                    }
                }
                (held, _) => *held = None,
            }
        }
    }
}

/// What one session made, for the counts.
#[derive(Default)]
struct Session {
    selections: usize,
    camouflages: usize,
    contexts: usize,
    parts: Vec<(&'static str, u64)>,
}

/// One session's answer before it is published.
struct Answer {
    meshes: Vec<(u64, Mesh)>,
    made: Session,
    /// Stacks chosen again.
    chosen: usize,
    refused: Option<String>,
    clear: bool,
}
