
use crate::{
    answer::{self, Message, State, Table},
    head,
    evaluate::{self, Find, Reasons, Scratch},
    live_rows::{self, LiveRows},
    query::{Query, Rows, TagRow, Tags, identity_until, tags_at},
    table_name,
};
use playhead::{
    Asked, Declared, Hook, Lanes, LogReads, Ordinal, Step, Wanted,
    changes::{Changes, Written},
    lane::Lane,
};
use replay_columnar::{manifest::Status, store::Value};
use replay_data::Hour;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io, mem,
    sync::{
        Arc, Mutex, MutexGuard, PoisonError,
        atomic::{AtomicU32, Ordering},
    },
    time::Instant,
};

pub type Sink = Box<dyn FnMut(&StepStats, Option<Message>) + Send>;

type Declare = Box<dyn Fn(Vec<Declared>, bool) -> io::Result<()> + Send + Sync>;

#[derive(Clone, Copy, Debug, Default)]
pub struct StepStats {
    pub epoch: u64,
    pub session: u64,
    pub first: Hour,
    pub last: Hour,
    pub reset: bool,
    pub gap: bool,
    pub touched: u32,
    pub evaluated: u32,
    pub changed: u32,
    pub cells: u32,
    pub rows_read: u64,
    pub sent: u32,
    pub messages: u32,
    pub bytes: u32,
    pub ns: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Counters {
    pub steps: u64,
    pub touched: u64,
    pub evaluated: u64,
    pub changed: u64,
    pub sent: u64,
    pub messages: u64,
    pub bytes: u64,
    pub ns: u64,
    pub most_ns: u64,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Held {
    pub queries: usize,
    pub waiting: usize,
    pub sent: usize,
    pub pending: usize,
    pub queries_bytes: usize,
    pub index: usize,
    pub total: usize,
}

enum Op {
    Follow(u32, Box<Query>),
    Unfollow(u32),
}

struct Reads {
    table: usize,
    columns: Vec<usize>,
    appended: bool,
    listed: bool,
}

struct Control {
    products: BTreeMap<u32, Vec<Declared>>,
    ops: Vec<Op>,
    followed: BTreeMap<u32, Reads>,
    declare: Option<Declare>,
    logs: Option<LogReads>,
    classes: BTreeMap<usize, bool>,
    session: u64,
}

type ProductHook = Arc<dyn Fn(&Step, &Lanes<'_>) + Send + Sync>;

struct Shared {
    page_seek: Mutex<(u64, u32)>,
    products: Mutex<Option<ProductHook>>,
    clock: i32,
    route: u32,
    control: Mutex<Control>,
    report: Mutex<(Counters, Held)>,
    wake: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

impl Shared {
    fn notify_wake(&self) {
        let wake = lock(&self.wake).clone();
        if let Some(wake) = wake {
            wake();
        }
    }
}

fn lock<T>(held: &Mutex<T>) -> MutexGuard<'_, T> {
    held.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Clone)]
pub struct Follow {
    shared: Arc<Shared>,
}

impl Follow {
    pub fn clock(&self) -> i32 {
        self.shared.clock
    }

    pub fn new(clock: i32) -> Self {
        Self {
            shared: Arc::new(Shared {
                clock,
                products: Mutex::default(),
                page_seek: Mutex::default(),
                route: next_route(),
                control: Mutex::new(Control { products: BTreeMap::new(), ops: Vec::new(), followed: BTreeMap::new(), declare: None, logs: None, classes: BTreeMap::new(), session: 0 }),
                report: Mutex::default(),
                wake: Mutex::default(),
            }),
        }
    }

    pub(crate) fn set_wake(&self, wake: Arc<dyn Fn() + Send + Sync>) {
        *lock(&self.shared.wake) = Some(wake);
    }

    pub(crate) fn has_work(&self) -> bool {
        let control = lock(&self.shared.control);
        !control.followed.is_empty() || !control.products.is_empty() || !control.ops.is_empty()
    }

    pub fn route(&self) -> u32 {
        self.shared.route
    }

    pub fn hook(&self, sink: Sink) -> Hook {
        let mut engine = Engine::new(Arc::clone(&self.shared), sink);
        Box::new(move |step: &Step, lanes: &Lanes<'_>| {
            engine.step(step, lanes);
            let products = lock(&engine.shared.products).clone();
            if let Some(products) = products { products(step, lanes); }
        })
    }

    pub(crate) fn anchor(&self, seek: u32, apply: impl FnOnce() -> u64) {
        {
            let mut page = lock(&self.shared.page_seek);
            *page = (apply(), seek);
        }
        self.shared.notify_wake();
    }

    pub(crate) fn product_hook(&self, hook: ProductHook) {
        *lock(&self.shared.products) = Some(hook);
    }

    pub(crate) fn rehooker(&self) -> crate::panel::Wake {
        let shared = Arc::downgrade(&self.shared);
        Arc::new(move || {
            if let Some(shared) = shared.upgrade() {
                let _ = redeclare(&mut lock(&shared.control), false);
                shared.notify_wake();
            }
        })
    }

    pub(crate) fn product_lanes(&self, id: u32, lanes: Vec<Declared>) -> io::Result<()> {
        let result = {
            let mut control = lock(&self.shared.control);
            if lanes.is_empty() { control.products.remove(&id); } else { control.products.insert(id, lanes); }
            redeclare(&mut control, false)
        };
        if result.is_ok() {
            self.shared.notify_wake();
        }
        result
    }

    pub fn bind(&self, declare: impl Fn(Vec<Declared>, bool) -> io::Result<()> + Send + Sync + 'static, logs: LogReads) -> io::Result<()> {
        let mut control = lock(&self.shared.control);
        control.logs = Some(logs);
        control.classes.clear();
        declare(union(&mut control)?, !control.followed.is_empty())?;
        control.declare = Some(Box::new(declare));
        Ok(())
    }

    pub fn follow(&self, id: u32, query: Query) -> io::Result<()> {
        query.check()?;
        let result = {
            let mut control = lock(&self.shared.control);
            control.followed.insert(id, Reads { table: query.table(), columns: query.declared(), appended: query.appended(), listed: query.listed() });
            control.ops.push(Op::Follow(id, Box::new(query)));
            redeclare(&mut control, true)
        };
        if result.is_ok() {
            self.shared.notify_wake();
        }
        result
    }

    pub fn serve(&self, session: u64) {
        lock(&self.shared.control).session = session;
        self.shared.notify_wake();
    }

    pub fn unfollow(&self, id: u32) -> io::Result<()> {
        let changed = {
            let mut control = lock(&self.shared.control);
            if control.followed.remove(&id).is_some() {
                control.ops.push(Op::Unfollow(id));
                redeclare(&mut control, false)?;
                true
            } else {
                false
            }
        };
        if changed {
            self.shared.notify_wake();
        }
        Ok(())
    }

    pub fn declared(&self) -> io::Result<Vec<Declared>> {
        union(&mut lock(&self.shared.control))
    }

    pub fn counters(&self) -> Counters {
        lock(&self.shared.report).0
    }

    pub fn held(&self) -> Held {
        lock(&self.shared.report).1
    }
}

fn next_route() -> u32 {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed) & answer::ROUTES
}

fn redeclare(control: &mut Control, joining: bool) -> io::Result<()> {
    let tables = union(control)?;
    match &control.declare {
        Some(declare) => declare(tables, joining),
        None => Ok(()),
    }
}

fn union(control: &mut Control) -> io::Result<Vec<Declared>> {
    let mut tables: BTreeMap<usize, (BTreeSet<usize>, bool, bool)> = BTreeMap::new();
    for reads in control.followed.values().filter(|reads| reads.table != head::TABLE) {
        let entry = tables.entry(reads.table).or_default();
        entry.0.extend(&reads.columns);
        entry.1 |= reads.appended;
        entry.2 |= reads.appended || reads.listed;
    }
    for lane in control.products.values().flatten() {
        let entry = tables.entry(lane.table).or_default();
        entry.0.extend(&lane.columns);
        entry.2 |= lane.keyed;
    }
    let mut declared = Vec::with_capacity(tables.len());
    for (table, (columns, feed, keyed)) in tables {
        let log = match (&control.logs, control.classes.get(&table)) {
            (_, Some(&log)) => log,
            (Some(logs), None) => *control.classes.entry(table).or_insert(logs.is_log(table)?),
            (None, None) => false,
        };
        declared.push(Declared { table, columns: columns.into_iter().collect(), keyed, passing: feed && !log, hourly: false });
    }
    Ok(declared)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Waiting,
    Active,
    Refused,
}

struct Followed {
    id: u32,
    query: Query,
    table: usize,
    declared: Vec<usize>,
    phase: Phase,
    dirty: bool,
    keys: Vec<u128>,
    tags: Tags,
    until: Hour,
    sent: Vec<u8>,
    has_sent: bool,
    pending: Vec<u8>,
    has_pending: bool,
    pending_hour: Hour,
    pending_held: u32,
    queued: bool,
    live: LiveRows,
    ranking: evaluate::Ranking,
    log: Table,
    standing: BTreeMap<Ordinal, Option<Listed>>,
    asked: Option<Asked>,
    unread: Option<String>,
}

type Listed = (u128, Box<[(i64, State)]>);

impl Followed {
    fn bytes(&self) -> (usize, usize, usize) {
        let log = self.log.keys.capacity() * 16 + self.log.values.capacity() * 8 + self.log.states.capacity() + (self.log.reasons.capacity() + self.log.names.capacity()) * 4 + self.log.hours.capacity() * 4;
        let standing = self.standing.len() * (size_of::<Ordinal>() + size_of::<Option<Listed>>() + 16) + self.standing.values().flatten().map(|(_, cells)| cells.len() * size_of::<(i64, State)>()).sum::<usize>();
        let query = match &self.query {
            Query::Cells { columns, rows, .. } => columns.capacity() * 8 + if let Rows::Keys(keys) | Rows::Live(keys) = rows { keys.capacity() * 16 } else { 0 },
            Query::Series { countries, .. } => countries.iter().map(|country| country.tag().map_or(0, str::len) + size_of::<dashboard_query::identity::Country>()).sum(),
            Query::Head(_) => 0,
        };
        (self.sent.capacity() + self.live.bytes(), self.pending.capacity() + log, size_of::<Self>() + query + self.ranking.bytes() + standing + self.keys.capacity() * 16 + self.declared.capacity() * 8)
    }
}

#[derive(Default)]
struct Readers {
    broad: Vec<usize>,
    keyed: Vec<usize>,
}

struct Watch {
    slot: Option<u32>,
    queries: Vec<usize>,
}

#[derive(Default)]
struct Index {
    queries: Vec<usize>,
    readers: HashMap<usize, Readers>,
    broad: Vec<usize>,
    series: Vec<usize>,
    logs: Vec<usize>,
    standing: Vec<usize>,
    watch: HashMap<u128, Watch>,
    at_slot: HashMap<u32, u128>,
    bits: Vec<u64>,
    unresolved: bool,
    ordered: Option<Vec<(u128, u32)>>,
    until: Hour,
    status: Option<Status>,
    withheld: bool,
}

impl Index {
    fn bit(&self, slot: u32) -> bool {
        self.bits.get(slot as usize / 64).is_some_and(|word| word & 1 << (slot % 64) != 0)
    }

    fn set(&mut self, slot: u32, key: u128) {
        let word = slot as usize / 64;
        if self.bits.len() <= word {
            self.bits.resize(word + 1, 0);
        }
        self.bits[word] |= 1 << (slot % 64);
        self.at_slot.insert(slot, key);
    }

    fn clear(&mut self, slot: u32) {
        if let Some(word) = self.bits.get_mut(slot as usize / 64) {
            *word &= !(1 << (slot % 64));
        }
        self.at_slot.remove(&slot);
    }

    fn watch(&mut self, query: usize, key: u128) {
        let watch = self.watch.entry(key).or_insert_with(|| Watch { slot: None, queries: Vec::new() });
        if watch.slot.is_none() {
            self.unresolved = true;
        }
        if !watch.queries.contains(&query) {
            watch.queries.push(query);
        }
    }

    fn unwatch(&mut self, query: usize, key: u128) {
        let Some(watch) = self.watch.get_mut(&key) else { return };
        watch.queries.retain(|&held| held != query);
        if watch.queries.is_empty() {
            let slot = watch.slot;
            self.watch.remove(&key);
            if let Some(slot) = slot {
                self.clear(slot);
            }
        }
    }

    fn resolve(&mut self, lane: &Lane) {
        if !self.unresolved {
            return;
        }
        self.unresolved = false;
        let cells = lane.cells();
        let mut found = Vec::new();
        for slot in cells.rows() {
            if let Some(key) = cells.key(slot)
                && let Some(watch) = self.watch.get_mut(&key)
                && watch.slot.is_none()
            {
                watch.slot = Some(slot);
                found.push((slot, key));
            }
        }
        for (slot, key) in found {
            self.set(slot, key);
        }
    }

    fn forget_slots(&mut self) {
        self.at_slot.clear();
        self.bits.clear();
        for watch in self.watch.values_mut() {
            watch.slot = None;
        }
        self.unresolved = !self.watch.is_empty();
        self.ordered = None;
    }

    fn ordered(&mut self, lane: &Lane) {
        if self.ordered.is_none() {
            let cells = lane.cells();
            let mut ordered: Vec<(u128, u32)> = cells.rows().filter_map(|slot| Some((cells.key(slot)?, slot))).collect();
            ordered.sort_unstable();
            self.ordered = Some(ordered);
        }
    }

    fn bytes(&self) -> usize {
        let lists = |lists: &[&Vec<usize>]| lists.iter().map(|list| list.capacity() * 8).sum::<usize>();
        lists(&[&self.queries, &self.broad, &self.series, &self.logs, &self.standing])
            + self.readers.capacity() * (8 + size_of::<Readers>() + 1)
            + self.readers.values().map(|readers| lists(&[&readers.broad, &readers.keyed])).sum::<usize>()
            + self.watch.capacity() * (16 + size_of::<Watch>() + 1)
            + self.watch.values().map(|watch| watch.queries.capacity() * 8).sum::<usize>()
            + self.at_slot.capacity() * (4 + 16 + 1)
            + self.bits.capacity() * 8
            + self.ordered.as_ref().map_or(0, |ordered| ordered.capacity() * 32)
    }
}

impl Find for Index {
    fn slot(&self, key: u128) -> Option<u32> {
        self.watch.get(&key).and_then(|watch| watch.slot)
    }

    fn ordered(&self) -> &[(u128, u32)] {
        self.ordered.as_deref().unwrap_or(&[])
    }
}

const GAP: &str = "No game hours are recorded here";

const HELD_EVERY: u64 = 64;

struct Engine {
    shared: Arc<Shared>,
    sink: Sink,
    slab: Vec<Option<Followed>>,
    free: Vec<usize>,
    ids: HashMap<u32, usize>,
    tables: BTreeMap<usize, Index>,
    waiting: Vec<usize>,
    heads: Vec<usize>,
    marked: Vec<usize>,
    queue: Vec<usize>,
    scratch: Scratch,
    reasons: Reasons,
    listed: Table,
    delta: Table,
    body: Vec<u8>,
    message: Vec<u8>,
    counters: Counters,
}

fn relist(shared: &Shared, followed: &mut Followed, lane: &Lane) {
    followed.standing.clear();
    followed.asked = None;
    followed.unread = None;
    let Some(log) = lane.log() else { return };
    let ordinals: Vec<Ordinal> = log.live().ones().collect();
    if ordinals.is_empty() {
        return;
    }
    followed.standing.extend(ordinals.iter().map(|&ordinal| (ordinal, None)));
    let logs = lock(&shared.control).logs.clone();
    match logs {
        Some(logs) => followed.asked = Some(logs.rows(Wanted { run: lane.run(), table: followed.table, ordinals, columns: followed.declared.clone() })),
        None => followed.unread = Some("Follow is bound to no playhead, which reads a log's listed rows".to_owned()),
    }
}

fn mark_all(slab: &mut [Option<Followed>], marked: &mut Vec<usize>, query: usize) {
    if let Some(followed) = &mut slab[query] {
        followed.live.touch_all();
    }
    mark(slab, marked, query);
}

fn mark_row(slab: &mut [Option<Followed>], marked: &mut Vec<usize>, query: usize, key: u128) {
    if let Some(followed) = &mut slab[query]
        && followed.query.live()
    {
        followed.live.touch(&followed.keys, key);
    }
    mark(slab, marked, query);
}

fn mark(slab: &mut [Option<Followed>], marked: &mut Vec<usize>, query: usize) {
    if let Some(followed) = &mut slab[query]
        && followed.phase == Phase::Active
        && !followed.dirty
    {
        followed.dirty = true;
        marked.push(query);
    }
}

fn offer(followed: &mut Followed, query: usize, body: &mut Vec<u8>, hour: Hour, queue: &mut Vec<usize>) -> bool {
    if followed.has_sent && followed.sent == *body {
        followed.has_pending = false;
        return false;
    }
    queue_body(followed, query, body, hour, queue);
    true
}

fn queue_body(followed: &mut Followed, query: usize, body: &mut Vec<u8>, hour: Hour, queue: &mut Vec<usize>) {
    mem::swap(&mut followed.pending, body);
    followed.has_pending = true;
    followed.pending_hour = hour;
    followed.pending_held = 0;
    if !followed.queued {
        followed.queued = true;
        queue.push(query);
    }
}

fn cells_of(table: &Table, body: &[u8]) -> u32 {
    if body.first() == Some(&answer::NOT_RECORDED) { 0 } else { (table.rows() * table.columns) as u32 }
}

fn holder_keys(tags: &[TagRow]) -> Vec<u128> {
    let mut keys: Vec<u128> = tags.iter().filter_map(|tag| if let TagRow::Key(key) = tag { Some(*key) } else { None }).collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

impl Engine {
    fn new(shared: Arc<Shared>, sink: Sink) -> Self {
        Self {
            shared,
            sink,
            slab: Vec::new(),
            free: Vec::new(),
            ids: HashMap::new(),
            tables: BTreeMap::new(),
            waiting: Vec::new(),
            heads: Vec::new(),
            marked: Vec::new(),
            queue: Vec::new(),
            scratch: Scratch::default(),
            reasons: Reasons::default(),
            listed: Table::default(),
            delta: Table::default(),
            body: Vec::new(),
            message: Vec::new(),
            counters: Counters::default(),
        }
    }

    fn step(&mut self, step: &Step, lanes: &Lanes<'_>) {
        let start = Instant::now();
        let mut stats = StepStats { epoch: step.epoch(), first: step.first(), last: step.last(), reset: step.reset(), gap: step.gap(), ..StepStats::default() };
        let (changed, session) = self.apply_ops();
        stats.session = session;
        if step.reset() {
            self.resend(step.last());
        }
        if step.gap() {
            self.answer_gap(step, &mut stats);
        } else {
            for changes in step.tables() {
                self.touch(changes, lanes, step.reset());
            }
            self.check_withheld(lanes);
            lanes.iter().for_each(|lane| self.reasons.learn(lane));
            self.follow_holders(lanes);
            self.activate(lanes, &mut stats);
            self.fill();
            stats.touched += self.marked.len() as u32;
            self.evaluate(step, lanes, &mut stats);
        }
        self.answer_heads(step, &mut stats);
        let message = self.send(step, lanes, &mut stats);
        let held = (changed || step.reset() || self.counters.steps.is_multiple_of(HELD_EVERY)).then(|| self.held());
        stats.ns = start.elapsed().as_nanos() as u64;
        let counters = &mut self.counters;
        counters.steps += 1;
        counters.touched += u64::from(stats.touched);
        counters.evaluated += u64::from(stats.evaluated);
        counters.changed += u64::from(stats.changed);
        counters.sent += u64::from(stats.sent);
        counters.messages += u64::from(stats.messages);
        counters.bytes += u64::from(stats.bytes);
        counters.ns += stats.ns;
        counters.most_ns = counters.most_ns.max(stats.ns);
        let mut report = lock(&self.shared.report);
        report.0 = self.counters;
        if let Some(held) = held {
            report.1 = held;
        }
        drop(report);
        (self.sink)(&stats, message);
    }

    fn apply_ops(&mut self) -> (bool, u64) {
        let (ops, session) = {
            let mut control = lock(&self.shared.control);
            (mem::take(&mut control.ops), control.session)
        };
        let changed = !ops.is_empty();
        for op in ops {
            match op {
                Op::Follow(id, query) => {
                    self.remove(id);
                    self.add(id, *query);
                }
                Op::Unfollow(id) => self.remove(id),
            }
        }
        (changed, session)
    }

    fn resend(&mut self, hour: Hour) {
        self.queue.clear();
        for (query, held) in self.slab.iter_mut().enumerate() {
            let Some(followed) = held else { continue };
            let refusal = (followed.phase == Phase::Refused).then(|| {
                if followed.has_pending { mem::take(&mut followed.pending) } else { mem::take(&mut followed.sent) }
            });
            followed.queued = false;
            followed.has_pending = false;
            followed.has_sent = false;
            followed.log.clear(followed.declared.len(), true);
            followed.live.resend();
            if let Some(mut body) = refusal {
                queue_body(followed, query, &mut body, hour, &mut self.queue);
            }
        }
    }

    fn add(&mut self, id: u32, query: Query) {
        let followed = Followed {
            id,
            table: query.table(),
            declared: query.declared(),
            query,
            phase: Phase::Waiting,
            dirty: false,
            keys: Vec::new(),
            tags: Tags::default(),
            until: Hour::MAX,
            sent: Vec::new(),
            has_sent: false,
            pending: Vec::new(),
            has_pending: false,
            pending_hour: 0,
            pending_held: 0,
            queued: false,
            live: LiveRows::default(),
            ranking: evaluate::Ranking::default(),
            log: Table::default(),
            standing: BTreeMap::new(),
            asked: None,
            unread: None,
        };
        let at = match self.free.pop() {
            Some(at) => {
                self.slab[at] = Some(followed);
                at
            }
            None => {
                self.slab.push(Some(followed));
                self.slab.len() - 1
            }
        };
        self.ids.insert(id, at);
        match &mut self.slab[at] {
            Some(followed) if matches!(followed.query, Query::Head(_)) => {
                followed.phase = Phase::Active;
                self.heads.push(at);
            }
            _ => self.waiting.push(at),
        }
    }

    fn remove(&mut self, id: u32) {
        let Some(at) = self.ids.remove(&id) else { return };
        let followed = self.slab[at].take().expect("a followed query");
        self.free.push(at);
        self.waiting.retain(|&held| held != at);
        self.heads.retain(|&held| held != at);
        self.queue.retain(|&held| held != at);
        if followed.phase != Phase::Active {
            return;
        }
        let Some(index) = self.tables.get_mut(&followed.table) else { return };
        for key in &followed.keys {
            index.unwatch(at, *key);
        }
        for list in [&mut index.queries, &mut index.broad, &mut index.series, &mut index.logs, &mut index.standing] {
            list.retain(|&held| held != at);
        }
        for readers in index.readers.values_mut() {
            readers.broad.retain(|&held| held != at);
            readers.keyed.retain(|&held| held != at);
        }
        index.readers.retain(|_, readers| !readers.broad.is_empty() || !readers.keyed.is_empty());
        if index.queries.is_empty() {
            self.tables.remove(&followed.table);
        }
    }

    fn activate(&mut self, lanes: &Lanes<'_>, stats: &mut StepStats) {
        let mut at = 0;
        while at < self.waiting.len() {
            let query = self.waiting[at];
            let followed = self.slab[query].as_mut().expect("a waiting query");
            let Some(lane) = lanes.get(followed.table).filter(|lane| evaluate::holds(&followed.query, lane)) else {
                at += 1;
                continue;
            };
            self.waiting.swap_remove(at);
            let refused = match followed.query.listed() && lane.log().is_none() {
                true => Some(format!("{} holds state: what the game lists now is a log's, and a table holding state answers its live rows as all its rows", table_name(followed.table))),
                false => evaluate::refusal(&followed.query, lane.cells()),
            };
            if let Some(refused) = refused {
                followed.phase = Phase::Refused;
                self.body.clear();
                answer::not_recorded(&refused, &mut self.body);
                stats.evaluated += 1;
                stats.changed += u32::from(offer(followed, query, &mut self.body, evaluate::head(lane), &mut self.queue));
                continue;
            }
            followed.phase = Phase::Active;
            let head = evaluate::head(lane);
            let index = self.tables.entry(followed.table).or_insert_with(|| Index { until: Hour::MAX, status: Some(lane.status().0), withheld: lane.withheld().is_some(), ..Index::default() });
            index.queries.push(query);
            let keyed = match &followed.query {
                Query::Cells { rows: Rows::Keys(keys) | Rows::Live(keys), .. } => {
                    followed.keys = keys.clone();
                    true
                }
                Query::Cells { rows: Rows::Appended, .. } => {
                    index.logs.push(query);
                    followed.log.clear(followed.declared.len(), true);
                    continue;
                }
                Query::Cells { rows: Rows::Listed, .. } => {
                    index.standing.push(query);
                    relist(&self.shared, followed, lane);
                    let ready = followed.asked.is_none();
                    if ready {
                        mark(&mut self.slab, &mut self.marked, query);
                    }
                    continue;
                }
                Query::Cells { .. } => {
                    index.broad.push(query);
                    false
                }
                Query::Head(_) => continue,
                Query::Series { identity, countries, top, parts, .. } => {
                    followed.ranking.clear();
                    followed.tags = tags_at(&followed.query, head);
                    followed.until = identity_until(identity, countries, head);
                    index.until = index.until.min(followed.until);
                    index.series.push(query);
                    if top.is_some() || *parts {
                        index.broad.push(query);
                        false
                    } else {
                        followed.keys = holder_keys(&followed.tags.rows);
                        true
                    }
                }
            };
            for key in &followed.keys {
                index.watch(query, *key);
            }
            for &column in &followed.declared {
                let readers = index.readers.entry(column).or_default();
                if keyed { readers.keyed.push(query) } else { readers.broad.push(query) }
            }
            mark(&mut self.slab, &mut self.marked, query);
        }
    }

    fn touch(&mut self, changes: &Changes, lanes: &Lanes<'_>, seek: bool) {
        let table = changes.table();
        let Some(index) = self.tables.get_mut(&table) else { return };
        let Some(lane) = lanes.get(table) else { return };
        let (slab, marked) = (&mut self.slab, &mut self.marked);
        if changes.reset() {
            index.status = Some(lane.status().0);
            index.withheld = lane.withheld().is_some();
            index.forget_slots();
            for &query in &index.standing {
                relist(&self.shared, slab[query].as_mut().expect("a followed list"), lane);
            }
            for &query in &index.series {
                slab[query].as_mut().expect("a followed series").until = Hour::MIN;
            }
            index.until = Hour::MIN;
            if seek {
                for &query in &index.logs {
                    let followed = slab[query].as_mut().expect("a followed log");
                    followed.log.clear(followed.declared.len(), true);
                }
            }
            index.queries.iter().for_each(|&query| mark_all(slab, marked, query));
            return;
        }
        if let Some((status, _)) = changes.status()
            && index.status != Some(status)
        {
            index.status = Some(status);
            index.queries.iter().for_each(|&query| mark_all(slab, marked, query));
        }
        if let Some(log) = changes.log() {
            let head = evaluate::head(lane);
            let key = |place: u32| log.keys().map_or(0, |keys| keys[place as usize]);
            let cell = |written: Option<&Written>, place: u32| evaluate::number(written.map_or(Value::Unrecorded, |written| written.value(place as usize)));
            if !log.is_empty() {
                for &query in &index.logs {
                    let followed = slab[query].as_mut().expect("a followed log");
                    let columns: Vec<Option<&Written>> = followed.declared.iter().map(|&column| log.column(column)).collect();
                    for (place, _, hour) in log.rows() {
                        followed.log.row(key(place));
                        followed.log.hours.push(hour);
                        for &written in &columns {
                            let (value, state) = cell(written, place);
                            followed.log.cell(value, state);
                        }
                    }
                    followed.pending_hour = head;
                    if !followed.queued {
                        followed.queued = true;
                        self.queue.push(query);
                    }
                }
            }
            if !log.is_empty() || !log.forgotten().is_empty() {
                for &query in &index.standing {
                    let followed = slab[query].as_mut().expect("a followed list");
                    let columns: Vec<Option<&Written>> = followed.declared.iter().map(|&column| log.column(column)).collect();
                    for (place, ordinal, _) in log.rows() {
                        followed.standing.insert(ordinal, Some((key(place), columns.iter().map(|&written| cell(written, place)).collect())));
                    }
                    for run in log.forgotten() {
                        while let Some((&ordinal, _)) = followed.standing.range(run.from..run.to).next() {
                            followed.standing.remove(&ordinal);
                        }
                    }
                    mark(slab, marked, query);
                }
            }
            return;
        }
        let (deleted, inserted) = (changes.deleted(), changes.inserted());
        if !deleted.is_empty() || !inserted.is_empty() {
            index.ordered = None;
            index.broad.iter().for_each(|&query| mark(slab, marked, query));
        }
        for &slot in deleted.slots() {
            if index.bit(slot) {
                let key = index.at_slot[&slot];
                index.clear(slot);
                let watch = index.watch.get_mut(&key).expect("a watched key");
                watch.slot = None;
                watch.queries.iter().for_each(|&query| mark_row(slab, marked, query, key));
            }
        }
        if !inserted.is_empty() {
            let cells = lane.cells();
            for &slot in inserted.slots() {
                let Some(key) = cells.key(slot) else { continue };
                if let Some(watch) = index.watch.get_mut(&key) {
                    watch.slot = Some(slot);
                    watch.queries.iter().for_each(|&query| mark_row(slab, marked, query, key));
                    index.set(slot, key);
                }
            }
        }
        let passing = changes.passing();
        if !index.logs.is_empty() && (!inserted.is_empty() || !passing.is_empty()) {
            let cells = lane.cells();
            let (mut at, mut gone) = (0, 0);
            while at < inserted.len() || gone < passing.len() {
                let passed = at == inserted.len() || (gone < passing.len() && passing.born()[gone] < inserted.hours()[at]);
                let (key, hour) = if passed {
                    (passing.keys().map_or(0, |keys| keys[gone]), passing.born()[gone])
                } else {
                    let slot = inserted.slots()[at];
                    let Some(key) = cells.key(slot) else {
                        at += 1;
                        continue;
                    };
                    (key, inserted.hours()[at])
                };
                for &query in &index.logs {
                    let followed = slab[query].as_mut().expect("a followed log");
                    followed.log.row(key);
                    followed.log.hours.push(hour);
                    for &column in &followed.declared {
                        let value = if passed {
                            passing.columns().iter().find(|written| written.column() == column).map_or(Value::Unrecorded, |written| written.value(gone))
                        } else {
                            cells.value(column, inserted.slots()[at])
                        };
                        let (value, state) = evaluate::number(value);
                        followed.log.cell(value, state);
                    }
                    followed.pending_hour = evaluate::head(lane);
                    if !followed.queued {
                        followed.queued = true;
                        self.queue.push(query);
                    }
                }
                if passed {
                    gone += 1;
                } else {
                    at += 1;
                }
            }
        }
        for written in changes.columns() {
            if written.is_empty() {
                continue;
            }
            let column = written.column();
            let Some(readers) = index.readers.get(&column) else { continue };
            readers.broad.iter().for_each(|&query| mark(slab, marked, query));
            if readers.keyed.is_empty() || index.at_slot.is_empty() {
                continue;
            }
            for &slot in written.slots() {
                if !index.bit(slot) {
                    continue;
                }
                let key = index.at_slot[&slot];
                for &query in &index.watch[&key].queries {
                    if slab[query].as_ref().is_some_and(|followed| followed.declared.contains(&column)) {
                        mark_row(slab, marked, query, key);
                    }
                }
            }
        }
        if changes.renumber().is_some() {
            index.forget_slots();
            index.queries.iter().for_each(|&query| mark_all(slab, marked, query));
        }
    }

    fn check_withheld(&mut self, lanes: &Lanes<'_>) {
        let (slab, marked) = (&mut self.slab, &mut self.marked);
        for (&table, index) in &mut self.tables {
            let Some(lane) = lanes.get(table) else { continue };
            let withheld = lane.withheld().is_some();
            if index.withheld != withheld {
                index.withheld = withheld;
                index.queries.iter().for_each(|&query| mark_all(slab, marked, query));
            }
        }
    }

    fn fill(&mut self) {
        for index in self.tables.values() {
            for &query in &index.standing {
                let followed = self.slab[query].as_mut().expect("a followed list");
                let Some(answer) = followed.asked.as_ref().and_then(Asked::take) else { continue };
                followed.asked = None;
                match answer {
                    Ok(rows) => {
                        for (place, &ordinal) in rows.ordinals().iter().enumerate() {
                            if let Some(held @ None) = followed.standing.get_mut(&ordinal) {
                                *held = Some((rows.keys()[place], rows.columns().iter().map(|written| evaluate::number(written.value(place))).collect()));
                            }
                        }
                    }
                    Err(error) => followed.unread = Some(error.to_string()),
                }
                mark(&mut self.slab, &mut self.marked, query);
            }
        }
    }

    fn follow_holders(&mut self, lanes: &Lanes<'_>) {
        for (&table, index) in &mut self.tables {
            let Some(head) = lanes.get(table).and_then(Lane::head) else { continue };
            if head < index.until {
                continue;
            }
            let mut until = Hour::MAX;
            for at in 0..index.series.len() {
                let query = index.series[at];
                let followed = self.slab[query].as_mut().expect("a followed series");
                if head >= followed.until {
                    let Query::Series { identity, countries, top, parts, .. } = &followed.query else { continue };
                    followed.until = identity_until(identity, countries, head);
                    followed.ranking.clear();
                    followed.tags = tags_at(&followed.query, head);
                    let keys = if top.is_some() || *parts { Vec::new() } else { holder_keys(&followed.tags.rows) };
                    let old = mem::replace(&mut followed.keys, keys);
                    for key in old.iter().filter(|key| !followed.keys.contains(key)) {
                        index.unwatch(query, *key);
                    }
                    for key in followed.keys.iter().filter(|key| !old.contains(key)) {
                        index.watch(query, *key);
                    }
                    if !followed.dirty {
                        followed.dirty = true;
                        self.marked.push(query);
                    }
                }
                until = until.min(followed.until);
            }
            index.until = until;
        }
    }

    fn evaluate(&mut self, step: &Step, lanes: &Lanes<'_>, stats: &mut StepStats) {
        for query in mem::take(&mut self.marked) {
            let followed = self.slab[query].as_mut().expect("a marked query");
            followed.dirty = false;
            let Some(lane) = lanes.get(followed.table).filter(|lane| evaluate::holds(&followed.query, lane)) else { continue };
            if followed.query.listed() {
                if followed.asked.is_some() {
                    continue;
                }
                self.body.clear();
                let unread = followed.standing.values().any(Option::is_none).then(|| "The listed rows' cells were not read".to_owned());
                match followed.unread.clone().or(unread).or_else(|| self.reasons.unrecorded(lane)) {
                    Some(reason) => answer::not_recorded(&reason, &mut self.body),
                    None => {
                        self.listed.clear(followed.declared.len(), false);
                        for (key, cells) in followed.standing.values().flatten() {
                            self.listed.row(*key);
                            cells.iter().for_each(|&(value, state)| self.listed.cell(value, state));
                        }
                        self.listed.encode(&mut self.body);
                    }
                }
                stats.evaluated += 1;
                stats.changed += u32::from(offer(followed, query, &mut self.body, evaluate::head(lane), &mut self.queue));
                continue;
            }
            let index = self.tables.get_mut(&followed.table).expect("an active query's table");
            index.resolve(lane);
            if matches!(followed.query, Query::Cells { rows: Rows::All, .. }) || followed.query.parts() {
                index.ordered(lane);
            }
            stats.evaluated += 1;
            if let Query::Cells { columns, rows: Rows::Live(keys), .. } = &followed.query {
                if let Some(reason) = self.reasons.unrecorded(lane) {
                    followed.live.unrecorded();
                    self.body.clear();
                    answer::not_recorded(&reason, &mut self.body);
                    stats.changed += u32::from(offer(followed, query, &mut self.body, evaluate::head(lane), &mut self.queue));
                    continue;
                }
                let (answer, read) = followed.live.evaluate(keys, columns, lane.cells(), &*index, &mut self.delta, &mut self.body);
                stats.cells += read as u32;
                match answer {
                    live_rows::Answer::Same => followed.has_pending = false,
                    live_rows::Answer::Body => {
                        stats.changed += 1;
                        let changed = self.body.first() == Some(&answer::CHANGED);
                        queue_body(followed, query, &mut self.body, evaluate::head(lane), &mut self.queue);
                        if changed { followed.pending_held = followed.live.whole(); }
                    }
                }
                continue;
            }
            if let Some(rows) = followed.ranking.write(&followed.query, lane, step.table(followed.table), &followed.tags, &self.reasons, &mut self.scratch.table, &mut self.body) {
                stats.rows_read += rows;
            } else {
                evaluate::write(&followed.query, lane, &*index, &followed.tags, &self.reasons, &mut self.scratch, &mut self.body);
            }
            stats.cells += cells_of(&self.scratch.table, &self.body);
            stats.changed += u32::from(offer(followed, query, &mut self.body, evaluate::head(lane), &mut self.queue));
        }
    }

    fn answer_gap(&mut self, step: &Step, stats: &mut StepStats) {
        for (query, held) in self.slab.iter_mut().enumerate() {
            let Some(followed) = held else { continue };
            if followed.phase != Phase::Active || followed.query.appended() || followed.table == head::TABLE {
                continue;
            }
            self.body.clear();
            let reason = match &followed.query {
                Query::Series { identity, .. } if identity.timeline.at(step.last()).is_none() => identity.timeline.reason(step.last()),
                _ => GAP.to_owned(),
            };
            answer::not_recorded(&reason, &mut self.body);
            followed.live.unrecorded();
            stats.touched += 1;
            stats.evaluated += 1;
            stats.changed += u32::from(offer(followed, query, &mut self.body, step.last(), &mut self.queue));
        }
    }

    fn answer_heads(&mut self, step: &Step, stats: &mut StepStats) {
        for at in 0..self.heads.len() {
            let query = self.heads[at];
            let followed = self.slab[query].as_mut().expect("a head");
            let Query::Head(value) = &followed.query else { continue };
            self.body.clear();
            head::write(value, step.last(), step.reset(), &mut self.listed, &mut self.body);
            stats.touched += 1;
            stats.evaluated += 1;
            stats.changed += u32::from(offer(followed, query, &mut self.body, step.last(), &mut self.queue));
        }
    }

    fn send(&mut self, step: &Step, lanes: &Lanes<'_>, stats: &mut StepStats) -> Option<Message> {
        let (first_epoch, seek) = *lock(&self.shared.page_seek);
        if step.epoch() < first_epoch { return None; }
        answer::begin(&mut self.message, self.shared.route, self.shared.clock, step.last(), seek, step.reset());
        if !step.gap() {
            for (&table, index) in &self.tables {
                let head = lanes.get(table).and_then(Lane::head).unwrap_or(Hour::MIN);
                let queries = if head < step.last() { &index.queries } else { &index.standing };
                for &at in queries {
                    let q = self.slab[at].as_ref().expect("indexed reading");
                    let through = if q.asked.is_some() { Hour::MIN } else { head };
                    if through < step.last() { answer::push(&mut self.message, q.id, through, &[]); }
                }
            }
        }
        for query in self.queue.drain(..) {
            let followed = self.slab[query].as_mut().expect("a queued query");
            let body = if followed.query.appended() && followed.phase != Phase::Refused {
                if followed.log.rows() == 0 {
                    followed.queued = false;
                    continue;
                }
                self.body.clear();
                followed.log.encode(&mut self.body);
                &self.body
            } else {
                if !followed.has_pending {
                    followed.queued = false;
                    continue;
                }
                &followed.pending
            };
            answer::push_held(&mut self.message, followed.id, followed.pending_hour, body, if followed.query.appended() { 0 } else { followed.pending_held });
            stats.sent += 1;
            followed.queued = false;
            if followed.query.appended() && followed.phase != Phase::Refused {
                followed.log.clear(followed.declared.len(), true);
            } else {
                mem::swap(&mut followed.sent, &mut followed.pending);
                followed.has_sent = true;
                followed.has_pending = false;
                if followed.query.live() {
                    followed.live.sent(followed.sent.first().copied().unwrap_or(answer::NOT_RECORDED));
                }
            }
        }
        if stats.sent == 0 && !step.reset() && self.ids.is_empty() {
            return None;
        }
        stats.messages = 1;
        stats.bytes = self.message.len() as u32;
        Some(Message { hour: step.last(), reset: step.reset(), answers: stats.sent, bytes: mem::take(&mut self.message) })
    }

    fn held(&self) -> Held {
        let mut held = Held { waiting: self.waiting.len(), ..Held::default() };
        for followed in self.slab.iter().flatten() {
            let (sent, pending, query) = followed.bytes();
            held.queries += 1;
            held.sent += sent;
            held.pending += pending;
            held.queries_bytes += query;
        }
        held.queries_bytes += self.slab.capacity() * size_of::<Option<Followed>>() + self.ids.capacity() * (4 + 8 + 1);
        held.index = self.tables.values().map(Index::bytes).sum::<usize>()
            + self.tables.len() * (8 + size_of::<Index>())
            + (self.waiting.capacity() + self.marked.capacity() + self.queue.capacity() + self.free.capacity()) * 8;
        held.total = held.sent + held.pending + held.queries_bytes + held.index + self.body.capacity() + self.message.capacity();
        held
    }
}

#[cfg(test)]
#[test]
fn reset_discards_an_abandoned_append_and_pending_body() {
    let follow = Follow::new(0);
    let mut engine = Engine::new(follow.shared, Box::new(|_, _| {}));
    for (id, rows) in [(0, Rows::Appended), (1, Rows::All)] {
        engine.add(id, Query::Cells { table: 2, columns: vec![0], rows });
        let q = engine.slab[id as usize].as_mut().unwrap();
        q.queued = true;
        q.has_pending = true;
        q.pending_hour = 99;
        q.log.row(42);
        engine.queue.push(id as usize);
    }
    engine.resend(3);
    assert!(engine.queue.is_empty());
    assert!(engine.slab.iter().flatten().all(|q| !q.queued && !q.has_pending && q.log.rows() == 0));
}
