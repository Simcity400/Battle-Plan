//! Each admitted land stack's mesh, chosen by the units layer on a worker of its own over the rows
//! the playhead's rows worker makes at the head ([`Rows::telling`]), never a stepped copy of its own.
//! The frame hands the worker the stacks whose figure or selection a settle changed. A build starts
//! once the one before has answered, and only when stacks were handed or a table it reads was made
//! anew since the rows the last build read; the Clock is one, so each hour a session decides the
//! graphics pools' limits again, which may compare the date. The rows worker tells which
//! rows of each table it made anew, so the session finds the kept answers they reach without
//! comparing tables ([`Told`]): it chooses again the stacks handed and those whose kept answers
//! fell, every other keeping its mesh unasked. A whole settle waits for the rows at its hour and
//! the product made from them. A table the selection's scripts read beyond [`TABLES`] is learned
//! and declared: the stacks keep their meshes until its rows arrive, never a guess.

use super::models::Select;
use crate::{
    layers::rebuilt::{absent, refusal},
    playhead_host::{
        Entry,
        rows::{self as head, Cadence, HeldKeys, RowReader, Rows},
    },
};
use replay_data::{Hour, rows::TableId};
use replay_frame::frame::{DecodedFrame, SharedView, TableView, discovering};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    sync::{
        Arc, Mutex, PoisonError,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
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

/// The equipment tables, read by key ([`RowReader::declare_keyed`]): the rows of the divisions of
/// the stacks held alone, a few hundred of some sixty thousand.
pub const KEYED: [TableId; 2] = [TableId::DivisionEquipment, TableId::NewDivisionEquipment];

/// How long a whole settle waits for the rows at its hour, and for the product made from them.
const WHOLE: Duration = Duration::from_secs(10);

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

/// A stack the frame hands on: its figure asks a mesh, or no longer does.
pub enum Ask {
    Stack(u64, Asked),
    Gone(u64),
}

struct Job {
    serial: u64,
    epoch: u64,
    frame: Arc<DecodedFrame>,
    /// The tables it declared (those read by key among them), and the rows of them made anew since
    /// the rows of the job before.
    tables: Vec<TableId>,
    told: head::Told,
    /// The keys each table read by key holds in the frame.
    keys: HeldKeys,
    ours: u32,
    /// Every stack held before is dropped first.
    clear: bool,
    asks: Vec<Ask>,
}

/// One job's answer: the meshes that changed, stamped with the hour of the rows it read.
struct Product {
    serial: u64,
    epoch: u64,
    hour: Hour,
    clear: bool,
    meshes: Vec<(u64, Mesh)>,
    made: Made,
    /// Stacks chosen again.
    chosen: usize,
    /// The worker's time on the job, and by part of it.
    busy_ns: u64,
    parts: Vec<(&'static str, u64)>,
    /// Tables the selection read undeclared: declared now, their rows to come.
    learned: Vec<TableId>,
    /// The stacks handed wait for another build: the session read a table it had not declared.
    again: bool,
    refused: Option<String>,
}

/// What the frame took from the worker since last asked: builds started, products, stacks chosen
/// again, selections made new and looked up by their selection key, camouflages and country
/// contexts made, and the worker's time, by part of a job too; the stacks that entered and left
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

pub struct Meshes {
    reader: RowReader,
    /// The tables it reads, the learned ones included, those the recording holds nothing of left out.
    tables: Vec<TableId>,
    jobs: Option<Sender<Job>>,
    made: Receiver<Product>,
    /// Bumped by a clear: a product of an earlier one is dropped.
    epoch: u64,
    /// The newest build started: its serial, the rows it read and its viewer.
    sent: Option<(u64, Arc<DecodedFrame>, u32)>,
    /// The serial of the newest build answered.
    answered: u64,
    clear: bool,
    asks: Vec<Ask>,
    /// The stacks handed wait for another build.
    again: bool,
    /// The equipment tables it reads by key (those the recording holds), the oldest divisions of
    /// the stacks handed and not let go, and the set last declared of them.
    keyed: Vec<TableId>,
    divisions: BTreeSet<u64>,
    declared: Arc<BTreeSet<u64>>,
    /// Each stack's mesh by its oldest member, as the worker last answered, and why a stack has none.
    pub meshes: HashMap<u64, usize>,
    pub withheld: HashMap<u64, String>,
    /// The hour of the rows the meshes are made from.
    pub hour: Option<Hour>,
    pub refused: Option<String>,
    counts: Mutex<Counts>,
}

impl Meshes {
    /// The worker, reading `rows` of [`TABLES`] and the tables `learned` before.
    pub fn start(select: Arc<dyn Select>, rows: &Arc<Rows>, learned: &[TableId]) -> Self {
        let entry = Arc::clone(rows.entry());
        let mut tables: Vec<TableId> = TABLES.iter().chain(learned).copied().filter(|table| !KEYED.contains(table)).collect();
        tables.sort_unstable_by_key(|table| table.index());
        tables.dedup();
        let every: Vec<TableId> = tables.iter().chain(&KEYED).copied().collect();
        let absent = absent(&entry, &every);
        tables.retain(|table| !absent.contains(table));
        let keyed: Vec<TableId> = KEYED.into_iter().filter(|table| !absent.contains(table)).collect();
        let reader = rows.telling(&tables, Cadence::Ready);
        let declared = Arc::new(BTreeSet::new());
        for &table in &keyed {
            reader.declare_keyed(table, Arc::clone(&declared));
        }
        let (jobs, received) = mpsc::channel::<Job>();
        let (made, taken) = mpsc::channel::<Product>();
        let spawned = std::thread::Builder::new().name("unit meshes".into()).spawn(move || {
            viewer_foundation::worker_priority::background_priority();
            Run::new(select, entry, &absent).run(&received, &made);
        });
        let refused = spawned.err().map(|error| format!("The unit meshes' worker cannot start: {error}"));
        Self {
            reader,
            tables,
            jobs: refused.is_none().then_some(jobs),
            made: taken,
            epoch: 0,
            sent: None,
            answered: 0,
            clear: true,
            asks: Vec::new(),
            again: false,
            keyed,
            divisions: BTreeSet::new(),
            declared,
            meshes: HashMap::new(),
            withheld: HashMap::new(),
            hour: None,
            refused,
            counts: Mutex::default(),
        }
    }

    /// Every stack handed before is forgotten: the next build hands every stack anew, and what a
    /// build out answers is dropped.
    pub fn clear(&mut self) {
        self.clear = true;
        self.epoch += 1;
        self.asks.clear();
        self.divisions.clear();
    }

    /// Takes `asks` into the stacks asked: their oldest divisions are the keys.
    fn note(&mut self, asks: &[Ask]) {
        let counts = self.counts.get_mut().unwrap_or_else(PoisonError::into_inner);
        for ask in asks {
            match ask {
                Ask::Stack(oldest, _) if self.divisions.insert(*oldest) => {
                    counts.entering += 1;
                    viewer_foundation::view_record::add("units", "entering", 1);
                }
                Ask::Gone(oldest) if self.divisions.remove(oldest) => {
                    counts.leaving += 1;
                    viewer_foundation::view_record::add("units", "leaving", 1);
                }
                _ => {}
            }
        }
    }

    /// The rows of every stack asked are declared by key: those of a stack handed since come in
    /// the reader's next frame.
    fn declare_keys(&mut self) {
        if *self.declared == self.divisions {
            return;
        }
        self.declared = Arc::new(self.divisions.clone());
        for &table in &self.keyed {
            self.reader.declare_keyed(table, Arc::clone(&self.declared));
        }
    }

    /// Whether a frame whose tables read by key hold `keys` holds the rows `ask` reads: every
    /// stack asked, for a frame a whole settle waits for, when `ask` is none.
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

    /// Takes what the worker made, then starts a build for the viewer `ours` with `asks` and those
    /// waiting when one may start: none is out and stacks were handed or a table it reads was made
    /// anew. `whole` first waits for the rows at
    /// `applied` and for the product of any build out, then for the product of the build it starts.
    /// Whether a mesh changed.
    pub fn exchange(&mut self, applied: Hour, ours: u32, asks: Vec<Ask>, whole: bool) -> bool {
        if !asks.is_empty() {
            self.note(&asks);
            self.declare_keys();
        }
        self.asks.extend(asks);
        let mut changed = self.receive(false);
        if self.jobs.is_none() {
            return changed;
        }
        if whole {
            let until = Instant::now() + WHOLE;
            // The rows standing at its hour that hold the equipment of every division asked. A
            // whole settle wants them whatever is on the worker, so it says it is ready before each
            // wait: a frame made below the hour would otherwise be the last made for it.
            loop {
                let seen = self.reader.newest().map_or(0, |newest| newest.published);
                self.reader.ready();
                if self.reader.wait_through(applied, until.saturating_duration_since(Instant::now())).is_err() {
                    break;
                }
                // The newest frame stands at the hour too: a frame stands longer than any before it.
                if self.reader.frame_keyed().is_some_and(|(_, keys)| self.holds(&keys, None)) || self.reader.wait_newer(seen, until.saturating_duration_since(Instant::now())).is_none() {
                    break;
                }
            }
            changed |= self.receive(true);
        }
        if self.out() {
            return changed;
        }
        // No build out: it has finished with the rows it took.
        self.reader.ready();
        // The last build answered read the rows it took: they stand at every hour no step since
        // changed a table they hold.
        if !self.again
            && self.hour.is_some()
            && let Some(stands) = self.sent.as_ref().and_then(|(_, read, _)| self.reader.through(Arc::as_ptr(read)))
        {
            self.hour = self.hour.max(Some(stands));
        }
        let Some((newest, keys)) = self.reader.frame_keyed() else { return changed };
        let made_anew = self.sent.as_ref().is_none_or(|(_, read, _)| self.tables.iter().copied().chain(self.keyed.iter().copied()).any(|table| !SharedView::ptr_eq(&read.tables[table.index()], &newest.tables[table.index()])));
        let sendable = self.asks.iter().any(|ask| self.holds(&keys, Some(ask)));
        let asked = self.clear || self.again || sendable || self.sent.as_ref().is_none_or(|&(_, _, viewer)| viewer != ours);
        if !made_anew && !asked {
            // Nothing it reads was made anew: the meshes stand for these rows, through the hour no
            // step since changed them by.
            if self.hour.is_some() {
                let stands = self.reader.through(Arc::as_ptr(&newest)).unwrap_or(newest.hour).max(newest.hour);
                self.hour = self.hour.max(Some(stands));
            }
            return changed;
        }
        let Some((frame, told, keys)) = self.reader.take_keyed() else { return changed };
        // A stack whose rows the frame does not hold yet waits for the frame that does: never
        // chosen as if it had none. The keys are the taken frame's own.
        let (sendable, waiting): (Vec<Ask>, Vec<Ask>) = std::mem::take(&mut self.asks).into_iter().partition(|ask| self.holds(&keys, Some(ask)));
        self.asks = waiting;
        let serial = self.sent.as_ref().map_or(1, |(serial, ..)| serial + 1);
        let tables = self.tables.iter().copied().chain(self.keyed.iter().copied()).collect();
        let job = Job { serial, epoch: self.epoch, frame: Arc::clone(&frame), tables, told, keys, ours, clear: self.clear, asks: sendable };
        if self.jobs.as_ref().is_some_and(|jobs| jobs.send(job).is_ok()) {
            (self.sent, self.clear, self.again) = (Some((serial, frame, ours)), false, false);
            self.counts.get_mut().unwrap_or_else(PoisonError::into_inner).builds += 1;
        } else {
            self.stopped();
        }
        if whole {
            changed |= self.receive(true);
        }
        changed
    }

    /// Whether a build is out.
    fn out(&self) -> bool {
        self.sent.as_ref().is_some_and(|&(serial, ..)| serial > self.answered)
    }

    /// Takes every product made; `wait` waits for the one of the build out.
    fn receive(&mut self, wait: bool) -> bool {
        let mut changed = false;
        loop {
            let product = if wait && self.out() {
                match self.made.recv_timeout(WHOLE) {
                    Ok(product) => product,
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => {
                        self.stopped();
                        break;
                    }
                }
            } else {
                match self.made.try_recv() {
                    Ok(product) => product,
                    Err(_) => break,
                }
            };
            changed |= self.take(product);
        }
        changed
    }

    fn stopped(&mut self) {
        self.jobs = None;
        self.refused = Some("The unit meshes' worker stopped".into());
    }

    fn take(&mut self, product: Product) -> bool {
        self.answered = product.serial;
        if product.epoch != self.epoch {
            return false;
        }
        let counts = self.counts.get_mut().unwrap_or_else(PoisonError::into_inner);
        counts.products += 1;
        counts.chosen += product.chosen as u64;
        counts.selections += product.made.selections as u64;
        // Each stack chosen took its selection by its key from the kept answers, or made it new.
        let looked_up = (product.chosen as u64).saturating_sub(product.made.selections as u64);
        counts.looked_up += looked_up;
        viewer_foundation::view_record::add("units", "choices_looked_up", looked_up);
        viewer_foundation::view_record::add("units", "choices_made_new", product.made.selections as u64);
        counts.camouflages += product.made.camouflages as u64;
        counts.contexts += product.made.contexts as u64;
        counts.busy_ns += product.busy_ns;
        for &(name, ns) in &product.parts {
            match counts.parts.iter_mut().find(|(held, _)| *held == name) {
                Some((_, sum)) => *sum += ns,
                None => counts.parts.push((name, ns)),
            }
        }
        self.refused = product.refused;
        if !product.learned.is_empty() {
            self.tables.extend(product.learned);
            self.tables.sort_unstable_by_key(|table| table.index());
            self.tables.dedup();
            self.reader.declare(&self.tables);
        }
        // A build that waits for a learned table's rows answers only the stacks let go.
        self.again |= product.again;
        if !product.again && self.refused.is_none() {
            self.hour = Some(product.hour);
        }
        let mut changed = false;
        if product.clear {
            changed = !self.meshes.is_empty();
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
        changed
    }

    /// The tables it reads, the learned ones included.
    pub fn tables(&self) -> &[TableId] {
        &self.tables
    }

    /// Whether it has made no meshes yet: its first rows or its first build are on their way.
    pub fn waiting(&self) -> bool {
        self.hour.is_none() && self.refused.is_none()
    }

    /// What it took since the last call, with the stacks waiting, the keys declared and the rows
    /// the newest frame holds of them as they stand.
    pub fn take_counts(&self) -> Counts {
        let mut counts = std::mem::take(&mut *self.counts.lock().unwrap_or_else(PoisonError::into_inner));
        counts.waiting = self.asks.iter().filter(|ask| matches!(ask, Ask::Stack(..))).count() as u64;
        counts.keys = self.declared.len() as u64;
        counts.keyed_rows = self.reader.frame().map_or(0, |frame| {
            self.keyed.iter().map(|&table| match frame.tables[table.index()].held() {
                TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.len() as u64,
                _ => 0,
            }).sum()
        });
        counts
    }

    /// Its levels for the flight record ([`viewer_foundation::view_record`]): the stacks waiting for
    /// their rows, the keys declared, and the rows its newest frame holds of them.
    pub fn record_view(&self) {
        use viewer_foundation::view_record::set;
        set("units", "choices_waiting", self.asks.iter().filter(|ask| matches!(ask, Ask::Stack(..))).count() as u64);
        set("units", "keys_declared", self.declared.len() as u64);
        let rows = self.reader.frame().map_or(0, |frame| {
            self.keyed.iter().map(|&table| match frame.tables[table.index()].held() {
                TableView::Complete(rows) | TableView::Truncated { rows, .. } => rows.len() as u64,
                _ => 0,
            }).sum()
        });
        set("units", "rows_by_key", rows);
    }

    /// What it asks, holds and waits for, for a test's message.
    #[cfg(test)]
    pub fn state(&self) -> String {
        let held = self.reader.frame_keyed().map(|(frame, keys)| (frame.hour, self.reader.through(Arc::as_ptr(&frame)), keys));
        format!(
            "divisions asked {:?}, declared {:?}; the newest frame's hour, the hour it stands at and its keys {held:?}; asks waiting {}, a build out {}, again {}, hour {:?}, refused {:?}, tables arriving {:?}",
            self.divisions,
            self.declared,
            self.asks.len(),
            self.out(),
            self.again,
            self.hour,
            self.refused,
            self.reader.arriving()
        )
    }

    pub fn bytes(&self) -> usize {
        self.meshes.capacity() * (8 + 8 + 8) + self.withheld.capacity() * (8 + 24 + 8) + self.withheld.values().map(String::capacity).sum::<usize>() + self.divisions.len() * 24
    }
}

impl Drop for Meshes {
    fn drop(&mut self) {
        self.jobs = None;
    }
}

struct Run {
    select: Arc<dyn Select>,
    entry: Arc<Entry>,
    /// Each table the recording holds nothing of, read as one view that refuses it by name.
    absent: Vec<(TableId, SharedView)>,
    held: HashMap<u64, Asked>,
    /// Stacks handed and not chosen yet.
    asked: HashSet<u64>,
    kept: KeptChoices,
    /// The rows of the job before, and those its kept answers were kept at with the rows of each
    /// table made anew since (none: made whole).
    last: Option<Arc<DecodedFrame>>,
    kept_at: Option<DecodedFrame>,
    since: BTreeMap<TableId, (SharedView, Option<BTreeMap<u128, u64>>)>,
}

impl Run {
    fn new(select: Arc<dyn Select>, entry: Arc<Entry>, absent: &[TableId]) -> Self {
        let mut run = Self { select, entry, absent: Vec::new(), held: HashMap::new(), asked: HashSet::new(), kept: KeptChoices::default(), last: None, kept_at: None, since: BTreeMap::new() };
        run.refuse(absent);
        run
    }

    /// `tables` too are read as refused by name: the recording holds nothing of them.
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

    fn run(mut self, received: &Receiver<Job>, made: &Sender<Product>) {
        while let Ok(job) = received.recv() {
            let started = Instant::now();
            let mut product = self.answer(job);
            product.busy_ns = started.elapsed().as_nanos() as u64;
            if made.send(product).is_err() {
                return;
            }
        }
    }

    fn answer(&mut self, job: Job) -> Product {
        let Job { serial, epoch, frame, tables, told, keys, ours, clear, asks } = job;
        let mut product = Product { serial, epoch, hour: frame.hour, clear, meshes: Vec::new(), made: Made::default(), chosen: 0, busy_ns: 0, parts: Vec::new(), learned: Vec::new(), again: false, refused: None };
        let mut since = Instant::now();
        let mut lap = |parts: &mut Vec<(&'static str, u64)>, name: &'static str| {
            let now = Instant::now();
            parts.push((name, now.duration_since(since).as_nanos() as u64));
            since = now;
        };
        if clear {
            self.held.clear();
            self.asked.clear();
        }
        for ask in asks {
            match ask {
                Ask::Stack(oldest, asked) => {
                    self.held.insert(oldest, asked);
                    self.asked.insert(oldest);
                }
                Ask::Gone(oldest) => {
                    self.held.remove(&oldest);
                    self.asked.remove(&oldest);
                    product.meshes.push((oldest, Ok(None)));
                }
            }
        }
        self.note(&frame, &tables, told);
        lap(&mut product.parts, "asks_and_told");
        let mut rows = DecodedFrame::clone(&frame);
        for (table, view) in &self.absent {
            rows.tables[table.index()] = view.clone();
        }
        let told = Told::new(self.since.iter().filter_map(|(&table, (from, rows))| Some((table, from.clone(), rows.as_ref()?.iter().map(|(&key, &fields)| (key, fields)).collect()))).collect());
        lap(&mut product.parts, "rows_and_told");
        let mut session = Vec::new();
        let keyed = keys.of(TableId::DivisionEquipment).or_else(|| keys.of(TableId::NewDivisionEquipment)).map(|keys| &**keys);
        let (chosen, missed) = discovering(|| self.select.choose_meshes(&rows, ours, (&self.kept, keyed), &self.held, &self.asked, &told, &mut |name, ns| session.push((name, ns))));
        product.parts.extend(session);
        match chosen {
            Ok(Chosen { meshes, made }) => {
                // The session ran: its answers are kept at these rows.
                (self.kept_at, self.since) = (Some(rows), BTreeMap::new());
                product.made = made;
                let missed: Vec<TableId> = missed.into_iter().filter(|table| !tables.contains(table) && !self.absent.iter().any(|(held, _)| held == table)).collect();
                if !missed.is_empty() {
                    let gone = absent(&self.entry, &missed);
                    self.refuse(&gone);
                    product.learned = missed.into_iter().filter(|table| !gone.contains(table)).collect();
                    product.again = true;
                    return product;
                }
                self.asked.clear();
                product.chosen = meshes.len();
                product.meshes.extend(meshes);
            }
            Err(error) => product.refused = Some(format!("The land unit models cannot be chosen at the head: {error}")),
        }
        product
    }

    /// Adds the rows of `tables` made anew since the job before, `told`, to those made anew since
    /// the rows the kept answers were kept at; a table whose rows differ untold counts as made whole.
    fn note(&mut self, frame: &Arc<DecodedFrame>, tables: &[TableId], mut told: head::Told) {
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
