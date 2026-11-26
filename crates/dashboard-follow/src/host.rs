
use crate::{
    Follow,
    live::{Group, GroupState, Live, cells_id},
    panel::{self, Panel, PanelData, PanelReader, PanelRows},
    push::{PANEL, Push},
    registry::{Anchor, Hosted, lacking},
};
use serde_json::Value;
use replay_frame::frame::DecodedFrame;
use std::collections::{BTreeMap, BTreeSet};
use playhead::{
    Declared, Playhead, Reader, Watcher,
    pages::{Pages, Run},
};
use replay_columnar::{
    manifest::Status,
    unit::{BLOCK, reader},
};
use replay_data::Hour;
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Condvar, Mutex, OnceLock, PoisonError, mpsc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

const FRAME: Duration = Duration::from_micros(16_667);
const PANEL_POLL: Duration = Duration::from_millis(8);

struct WaitGate {
    version: AtomicU64,
    state: Mutex<bool>,
    cv: Condvar,
}

impl WaitGate {
    fn new() -> Self {
        Self { version: AtomicU64::new(0), state: Mutex::new(false), cv: Condvar::new() }
    }

    fn stamp(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    fn wake(&self) {
        self.version.fetch_add(1, Ordering::Release);
        let _guard = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        drop(_guard);
        self.cv.notify_all();
    }

    fn stop(&self) {
        self.version.fetch_add(1, Ordering::Release);
        let mut stopped = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        *stopped = true;
        drop(stopped);
        self.cv.notify_all();
    }

    fn wait(&self, seen: u64, timeout: Option<Duration>) -> bool {
        let mut stopped = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if *stopped {
            return true;
        }
        if self.version.load(Ordering::Acquire) != seen {
            return false;
        }
        stopped = match timeout {
            Some(dur) if !dur.is_zero() => self.cv.wait_timeout(stopped, dur).unwrap_or_else(PoisonError::into_inner).0,
            Some(_) => return false,
            None => self.cv.wait(stopped).unwrap_or_else(PoisonError::into_inner),
        };
        *stopped
    }
}

pub fn held_tables(pages: &dyn Pages, tables: usize) -> Vec<bool> {
    (0..tables)
        .map(|table| {
            let unwithheld = pages.kinds(table).is_some_and(|kinds| !kinds.is_empty())
                && pages.runs().iter().enumerate().any(|(run, held)| pages.withheld_at(run, table, held.last_hour()).is_none());
            let restored = pages.runs().iter().all(|run| run.pages.iter().filter_map(|page| page.tables.get(table)).any(|entry| entry.first.is_some() || entry.restore.is_some()));
            unwithheld && restored && pages.runs().iter().any(|run| recorded(run, table))
        })
        .collect()
}

fn recorded(run: &Run, table: usize) -> bool {
    run.pages.iter().enumerate().any(|(block, page)| {
        let Some(entry) = page.tables.get(table) else { return false };
        let first = run.first_hour + (block as u32 * BLOCK) as Hour;
        let mut hour = 0;
        loop {
            match reader::status(entry, hour, first) {
                Ok((status, ..)) if status != Status::NotRecorded => return true,
                Ok((_, _, _, Some(next))) if next > hour => hour = next,
                _ => return false,
            }
        }
    })
}

struct Shown {
    panel: Box<dyn Panel>,
    reader: Box<dyn PanelReader>,
    frame: Option<Arc<DecodedFrame>>,
    data: Option<PanelData>,
    pushed: Option<Hour>,
    version: (u64, u32),
    class: (u32, u32),
}

impl Shown {
    fn new(panel: Box<dyn Panel>, reader: Box<dyn PanelReader>, class: (u32, u32)) -> Self {
        Self { panel, reader, frame: None, data: None, pushed: None, version: (0, 0), class }
    }
}

fn release(ids: &mut Vec<(Group, u32)>, cells: Option<&HostCells>) {
    if let Some(cells) = cells {
        for (_, id) in ids.drain(..) {
            let _ = cells.follow.unfollow(id);
        }
    }
    ids.clear();
}

#[derive(Clone)]
struct HostCells {
    follow: Follow,
    held: Vec<bool>,
}

fn register(group: &Group, ids: &mut Vec<(Group, u32)>, kept: &mut Vec<(Group, u32)>, cells: Option<&HostCells>, class: (u32, u32)) -> Result<u32, String> {
    let Some(cells) = cells else { return Err("This playhead follows no live cells".into()) };
    if let Some(reason) = lacking(group.table.index(), cells.held.get(group.table.index()).copied().unwrap_or(false)) {
        return Err(reason);
    }
    if let Some(at) = ids.iter().position(|(held, _)| held == group) {
        let (held, id) = ids.swap_remove(at);
        kept.push((held, id));
        return Ok(id);
    }
    let id = cells_id(class.0, class.1);
    match cells.follow.follow(id, group.query()) {
        Ok(()) => {
            kept.push((group.clone(), id));
            Ok(id)
        }
        Err(error) => Err(error.to_string()),
    }
}

fn with_cells(frame: &DecodedFrame, data: &PanelData, live: Option<&Live>, ids: &mut Vec<(Group, u32)>, cells: Option<&HostCells>, class: (u32, u32)) -> PanelData {
    let (Ok(reply @ Value::Object(_)), Some(live)) = (data, live.filter(|live| !live.is_empty())) else {
        release(ids, cells);
        return data.clone();
    };
    let mut kept = Vec::with_capacity(live.groups.len());
    let states: Vec<GroupState> = live
        .groups
        .iter()
        .map(|group| {
            match register(group, ids, &mut kept, cells, class) {
                Ok(id) => match group.seed(frame) {
                    Some(values) => GroupState::Seeded { follow: id, hour: frame.hour, values },
                    None => GroupState::Follow(id),
                },
                Err(reason) => GroupState::Unavailable(reason),
            }
        })
        .collect();
    release(ids, cells);
    *ids = kept;
    let mut reply = reply.clone();
    if let Some(fields) = reply.as_object_mut() {
        fields.insert("live".into(), live.json(&states));
    }
    Ok(reply)
}

fn run_panel(job: impl FnOnce() + Send + 'static) {
    type Job = Box<dyn FnOnce() + Send>;
    static JOBS: OnceLock<mpsc::Sender<Job>> = OnceLock::new();
    let jobs = JOBS.get_or_init(|| {
        let (send, receive) = mpsc::channel::<Job>();
        let receive = Arc::new(Mutex::new(receive));
        let threads = std::thread::available_parallelism().map_or(4, |n| (n.get() / 2).max(4));
        for i in 0..threads {
            let receive = receive.clone();
            std::thread::Builder::new().name(format!("follow panel {i}")).spawn(move || loop {
                let job = receive.lock().unwrap_or_else(PoisonError::into_inner).recv();
                match job { Ok(job) => job(), Err(_) => break }
            }).expect("a follow panel worker");
        }
        send
    });
    jobs.send(Box::new(job)).expect("follow panel workers");
}

struct Work<T> {
    state: Mutex<(Option<T>, bool)>,
}

impl<T> Work<T> {
    fn new(value: Option<T>) -> Self { Self { state: Mutex::new((value, false)) } }

    fn take(&self) -> Option<T> {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let value = state.0.take();
        if value.is_none() { state.1 = true; }
        value
    }

    fn put(&self, value: T) -> bool {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.0 = Some(value);
        std::mem::take(&mut state.1)
    }
}

#[derive(Default)]
struct Panels {
    stamp: (i32, u32, u32, u32),
    products: BTreeMap<u32, Arc<Work<StepProduct>>>,
    due: BTreeSet<(Hour, u32)>,
    stream: Option<(u64, u32, u64, Option<usize>)>,
    resend: Option<Hour>,
    pace: Option<(bool, f64)>,
    rows: Option<Arc<dyn PanelRows>>,
    push: Option<Arc<Push>>,
    shown: BTreeMap<u32, Arc<Work<Shown>>>,
    frame_version: u64,
    thread: Option<JoinHandle<()>>,
    cells: Option<HostCells>,
    session: u64,
}

#[derive(Default)]
struct ProductReads {
    epoch: u64,
    seek: u32,
    tokens: BTreeMap<u32, Arc<AtomicBool>>,
    cells: BTreeMap<u32, Vec<(Group, u32)>>,
    host: Option<HostCells>,
}

impl ProductReads {
    fn current(&self, epoch: u64, seek: u32) -> bool { epoch >= self.epoch && seek == self.seek }

    fn anchor(&mut self, epoch: u64, seek: u32) {
        if (self.epoch, self.seek) == (epoch, seek) { return; }
        self.epoch = epoch;
        self.seek = seek;
        for token in self.tokens.values_mut() { std::mem::take(token).store(true, Ordering::Release); }
    }

    fn remove(&mut self, id: u32) {
        if let Some(token) = self.tokens.remove(&id) {
            token.store(true, Ordering::Release);
            if let Some(mut cells) = self.cells.remove(&id) { release(&mut cells, self.host.as_ref()); }
        }
    }

    fn register(&mut self, id: u32) {
        self.remove(id);
        self.tokens.insert(id, Arc::default());
    }
}

struct StepProduct {
    id: u32,
    pending_seed: Option<(u64, u32, Hour)>,
    started_on: Option<(u64, u32)>,
    read: panel::ProductReader,
    lanes: Vec<Declared>,
    wire: panel::product::Delivery,
    last: Option<(u64, u32, Hour)>,
    joined: bool,
    cells: Vec<(Group, u32)>,
    class: (u32, u32),
    host: Option<HostCells>,
    followed: Option<Arc<PanelData>>,
    input: Option<Arc<PanelData>>,
    resend: bool,
}

struct Seed {
    hour: Hour,
    stamp: (i32, u32, u32),
    session: u64,
    epoch: u64,
    token: Arc<AtomicBool>,
    reads: Arc<Mutex<ProductReads>>,
    push: Arc<Push>,
    wake: panel::Wake,
}

impl Seed {
    fn start<E: Send + Sync + 'static>(self, mut product: StepProduct, work: Arc<Work<StepProduct>>, owner: Arc<E>, get: fn(&E) -> &Playhead) {
        run_panel(move || {
            let product_id = product.id;
            product.publish_seed(self.stamp, self.session, self.hour, self.epoch, &self.token, &self.reads, &self.push, || get(&owner).epoch());
            work.put(product);
            if !self.token.load(Ordering::Acquire) || self.reads.lock().unwrap_or_else(PoisonError::into_inner).tokens.contains_key(&product_id) {
                (self.wake)();
            }
        });
    }
}

impl StepProduct {
    fn publish_seed(&mut self, stamp: (i32, u32, u32), session: u64, head: Hour, epoch: u64, token: &AtomicBool,
        reads: &Mutex<ProductReads>, push: &Push, current_epoch: impl Fn() -> u64) {
        let id = self.id;
        if token.load(Ordering::Acquire) || current_epoch() != epoch { return; }
        let (clock, route, seek) = stamp;
        self.started_on = Some((epoch, seek));
        self.joined = false;
        self.pending_seed = None;
        let Some(change) = self.read_at(seek, epoch, head, true, token, None, reads) else {
            self.pending_seed = Some((epoch, seek, head));
            return;
        };
        let packet = change.map(|(kind, body)| panel::dated::encoded(clock, route, seek, id, head, kind, body));
        let reads = reads.lock().unwrap_or_else(PoisonError::into_inner);
        if reads.current(epoch, seek) && !token.load(Ordering::Acquire) && current_epoch() == epoch
            && let Some(packet) = packet {
            push.offer_from(session, PANEL, packet);
            self.wire.commit();
        }
    }

    fn step(&mut self, input: panel::ProductStep<'_>, (clock, route, seek): (i32, u32, u32), session: u64, token: &AtomicBool,
        active: &Mutex<ProductReads>, push: &Push, playhead: &Playhead) -> (Hour, Option<(u8, Vec<u8>)>) {
        let step = input.step;
        let (epoch, hour, id) = (step.epoch(), step.last(), self.id);
        let current = || !token.load(Ordering::Acquire) && playhead.epoch() == epoch;
        if !self.joined && self.last == Some((epoch, seek, hour)) {
            self.joined = true;
            return (hour.saturating_add(1), Some((panel::dated::STANDS, Vec::new())));
        }
        if let Some((e, g, at)) = self.pending_seed {
            if (e, g) == (epoch, seek) && at < hour && self.last.is_none_or(|(le, lg, _)| (le, lg) != (e, g)) {
                let head = playhead.head(Instant::now()).clamp(at, hour);
                if head < hour {
                    let Some(change) = self.read_at(seek, epoch, head, true, token, Some(input), active) else {
                        self.resend = true;
                        return (hour, Some((panel::dated::WAITING, Vec::new())));
                    };
                    let guard = active.lock().unwrap_or_else(PoisonError::into_inner);
                    if guard.current(epoch, seek) && current() && let Some((kind, body)) = change {
                        push.offer_from(session, PANEL, panel::dated::encoded(clock, route, seek, id, head, kind, body));
                        self.wire.commit();
                    }
                }
            }
            self.pending_seed = None;
        }
        if !self.joined && let Some((e, g, mut h)) = self.last && (e, g) == (epoch, seek) && !step.reset() {
            let quantum = playhead.step_hours();
            while h.saturating_add(quantum) < step.first() && current() {
                h += quantum;
                let (kind, body) = self.read_at(seek, epoch, h, false, token, None, active).flatten().unwrap_or((panel::dated::STANDS, &[]));
                let guard = active.lock().unwrap_or_else(PoisonError::into_inner);
                if !guard.current(epoch, seek) || !current() { break; }
                push.offer_from(session, PANEL, panel::dated::encoded(clock, route, seek, id, h, kind, body));
                self.wire.commit();
            }
        }
        let joining = !self.joined;
        let Some(change) = self.read_at(seek, epoch, hour, step.reset(), token, Some(input), active) else {
            if self.last.is_none_or(|(e, g, _)| (e, g) != (epoch, seek)) && self.pending_seed.is_none() {
                self.pending_seed = Some((epoch, seek, hour));
                self.joined = false;
            }
            self.resend = true;
            return (hour, Some((panel::dated::WAITING, Vec::new())));
        };
        let change = change.map(|(kind, body)| (kind, body.to_vec())).or_else(|| joining.then(|| (panel::dated::STANDS, Vec::new())));
        self.wire.commit();
        self.joined = true;
        (hour.saturating_add(1), change)
    }

    fn read_at(&mut self, page_seek: u32, epoch: u64, hour: Hour, reset: bool, cancelled: &AtomicBool, step: Option<panel::ProductStep<'_>>, bindings: &Mutex<ProductReads>) -> Option<Option<(u8, &[u8])>> {
        let seed = reset || self.last.is_none_or(|(e, g, h)| e != epoch || g != page_seek || h > hour);
        let send = seed || std::mem::take(&mut self.resend);
        let held = self.host.as_ref().map(|host| host.held.clone());
        let step = step.map(|step| panel::ProductStep { held: held.as_deref(), ..step });
        let read = catch_unwind(AssertUnwindSafe(|| (self.read)(hour, send, cancelled, step)))
            .unwrap_or_else(|p| panel::ProductRead { value: Arc::new(Err(format!("Panel read at hour {hour} failed: {}", panicked(&*p)))),
                rows: 0, fresh: true, pending: false, live: None, lanes: Vec::new() });
        let mut active = bindings.lock().unwrap_or_else(PoisonError::into_inner);
        if cancelled.load(Ordering::Acquire) || !active.current(epoch, page_seek) { return None; }
        if !read.lanes.is_empty() { self.read_lanes(&read.lanes); }
        if read.pending { return None; }
        self.last = Some((epoch, page_seek, hour));
        self.cells = active.cells.remove(&self.id).unwrap_or_default();
        drop(active);
        let value = self.follow_cells(&read.value, read.live.as_deref(), hour, send, read.fresh);
        let mut current = bindings.lock().unwrap_or_else(PoisonError::into_inner);
        if cancelled.load(Ordering::Acquire) || !current.current(epoch, page_seek) {
            drop(current);
            release(&mut self.cells, self.host.as_ref());
            return None;
        }
        if !self.cells.is_empty() { current.cells.insert(self.id, std::mem::take(&mut self.cells)); }
        drop(current);
        Some(self.wire.update(send, &value))
    }

    fn read_lanes(&mut self, read: &[Declared]) {
        let Some(host) = &self.host else { return };
        let mut lanes = self.lanes.clone();
        for wanted in read.iter().filter(|wanted| host.held.get(wanted.table) == Some(&true)) {
            match lanes.iter_mut().find(|lane| lane.table == wanted.table) {
                Some(lane) => {
                    lane.columns.extend(&wanted.columns);
                    lane.columns.sort_unstable();
                    lane.columns.dedup();
                    lane.keyed |= wanted.keyed;
                    lane.passing |= wanted.passing;
                    lane.hourly |= wanted.hourly;
                }
                None => lanes.push(wanted.clone()),
            }
        }
        lanes.sort_by_key(|lane| lane.table);
        if lanes != self.lanes && host.follow.product_lanes(self.id, lanes.clone()).is_ok() {
            self.lanes = lanes;
        }
    }

    fn follow_cells(&mut self, value: &Arc<PanelData>, live: Option<&Live>, hour: Hour, seed: bool, fresh: bool) -> Arc<PanelData> {
        let (Ok(reply @ Value::Object(_)), Some(live)) = (&**value, live.filter(|live| !live.is_empty())) else {
            release(&mut self.cells, self.host.as_ref());
            self.followed = None;
            self.input = None;
            return value.clone();
        };
        if !seed && let (Some(input), Some(followed)) = (&self.input, &self.followed) && Arc::ptr_eq(input, value) { return followed.clone(); }
        self.input = Some(value.clone());
        let mut reply = reply.clone();
        let mut kept = Vec::with_capacity(live.groups.len());
        if let Some(groups) = reply.get_mut("live").and_then(|live| live.get_mut("groups")).and_then(Value::as_array_mut) {
            for (group, out) in live.groups.iter().zip(groups) {
                let Some(out) = out.as_object_mut() else { continue };
                let new = !self.cells.iter().any(|(held, _)| held == group);
                match register(group, &mut self.cells, &mut kept, self.host.as_ref(), self.class) {
                    Ok(id) => {
                        out.insert("follow".into(), id.into());
                        let exact = || out.get("values").and_then(Value::as_array).is_some_and(|values| values.iter().all(|value| value.as_i64().is_none_or(|n| n.unsigned_abs() < 1 << 53)));
                        if (new || seed) && fresh && exact() { out.insert("seedHour".into(), hour.into()); } else { out.remove("values"); out.remove("rawValues"); }
                    }
                    Err(reason) => {
                        out.remove("values");
                        out.remove("rawValues");
                        out.insert("unavailable".into(), reason.into());
                    }
                }
            }
        }
        release(&mut self.cells, self.host.as_ref());
        self.cells = kept;
        let made = Arc::new(Ok(reply));
        match &self.followed {
            Some(last) if **last == *made => last.clone(),
            _ => { self.followed = Some(made.clone()); made }
        }
    }
}

impl Panels {
    fn remove(&mut self, id: u32) {
        if self.products.remove(&id).is_some() {
            self.due.retain(|&(_, due)| due != id);
            if let Some(cells) = &self.cells { let _ = cells.follow.product_lanes(id, Vec::new()); }
        }
        self.shown.remove(&id);
    }
}

fn panicked(payload: &(dyn std::any::Any + Send)) -> String {
    payload.downcast_ref::<&str>().map(|text| (*text).to_owned()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_else(|| "a panic with no message".to_owned())
}

fn evaluate(id: u32, shown: &mut Shown, stamp: (i32, u32, u32, u32), version: u64, token: &AtomicBool,
    active: &Mutex<ProductReads>, push: &Push, session: u64, playhead: &Playhead) {
    use panel::dated::{self, Update};
    let (clock, route, epoch, seek) = stamp;
    let current = || !token.load(Ordering::Acquire) && playhead.epoch() as u32 == epoch;
    if shown.version != (version, epoch) {
        shown.frame = None;
        shown.data = None;
        shown.pushed = None;
        shown.version = (version, epoch);
    }
    if let Some((hour, reason)) = shown.reader.stopped() {
        let data = Err(reason);
        if shown.data.as_ref() != Some(&data) {
            let packet = dated::message(clock, route, seek, &[Update::Data(id, hour, &data)]);
            let mut reads = active.lock().unwrap_or_else(PoisonError::into_inner);
            if !current() || reads.seek != seek { return; }
            if let Some(mut cells) = reads.cells.remove(&id) { release(&mut cells, reads.host.as_ref()); }
            push.offer_from(session, PANEL, packet);
            shown.data = Some(data);
        }
        return;
    }
    let Some(frame) = shown.reader.frame() else { return };
    let Some((through, _)) = shown.reader.through_seek(&frame).filter(|&(_, at)| at as u32 == epoch) else { return };
    let mut updates = Vec::new();
    let data;
    if !shown.frame.as_ref().is_some_and(|last| Arc::ptr_eq(last, &frame)) {
        if shown.data.is_none() || !shown.panel.unchanged(&frame) {
            let made = catch_unwind(AssertUnwindSafe(|| shown.panel.data(&frame)))
                .unwrap_or_else(|p| Err(format!("This panel could not be made at hour {} ({})", frame.hour, panicked(&*p))));
            let (mut bound, host) = {
                let mut reads = active.lock().unwrap_or_else(PoisonError::into_inner);
                if !current() || reads.seek != seek { return; }
                (reads.cells.remove(&id).unwrap_or_default(), reads.host.clone())
            };
            data = with_cells(&frame, &made, shown.panel.live(), &mut bound, host.as_ref(), shown.class);
            let mut reads = active.lock().unwrap_or_else(PoisonError::into_inner);
            if !current() || reads.seek != seek {
                drop(reads);
                release(&mut bound, host.as_ref());
                return;
            }
            if !bound.is_empty() { reads.cells.insert(id, bound); }
            updates.push(Update::Data(id, frame.hour, &data));
            shown.data = Some(made);
            shown.pushed = Some(frame.hour);
        }
        shown.frame = Some(frame);
    }
    if shown.pushed.is_some_and(|last| through > last) {
        updates.push(Update::Stands(id, through));
        shown.pushed = Some(through);
    }
    if !updates.is_empty() {
        let packet = dated::message(clock, route, seek, &updates);
        let reads = active.lock().unwrap_or_else(PoisonError::into_inner);
        if current() && reads.seek == seek { push.offer_from(session, PANEL, packet); }
    }
}

pub struct PlayheadHost<E: Send + Sync + 'static> {
    owner: Arc<E>,
    panels: Arc<Mutex<Panels>>,
    reads: Arc<Mutex<ProductReads>>,
    get: fn(&E) -> &Playhead,
    anchored: fn(&E, Anchor),
    follow: Follow,
    reader: Reader,
    held: Vec<bool>,
    stop: Arc<AtomicBool>,
    driver_gate: Arc<WaitGate>,
    panel_gate: Arc<WaitGate>,
    panel_work: Arc<AtomicU64>,
    _driver_watch: Watcher,
    _panel_watch: Watcher,
    driver: Mutex<Option<JoinHandle<()>>>,
}

impl<E: Send + Sync + 'static> PlayheadHost<E> {
    pub fn new(owner: Arc<E>, get: fn(&E) -> &Playhead, anchored: fn(&E, Anchor), follow: Follow, held: Vec<bool>) -> std::io::Result<Arc<Self>> {
        let playhead = get(&owner);
        let reader = playhead.declare(Vec::new(), None)?;
        let mut consumer = playhead.consumer(reader);
        let bound = Arc::downgrade(&owner);
        follow.bind(move |tables, joining| {
            let Some(owner) = bound.upgrade() else { return Ok(()) };
            let playhead = get(&owner);
            playhead.redeclare(reader, tables)?;
            if joining { playhead.seed_paused_query(); }
            Ok(())
        }, playhead.log_reads())?;
        let stop = Arc::new(AtomicBool::new(false));
        let (driven, stopping) = (Arc::clone(&owner), Arc::clone(&stop));
        let panels = Arc::new(Mutex::new(Panels { stamp: (follow.clock(), follow.route(), playhead.epoch() as u32, 0), cells: Some(HostCells { follow: follow.clone(), held: held.clone() }), ..Panels::default() }));
        let weak = Arc::downgrade(&panels);
        let reads = Arc::new(Mutex::new(ProductReads { host: Some(HostCells { follow: follow.clone(), held: held.clone() }), ..Default::default() }));
        let active = reads.clone();
        let product_owner = Arc::downgrade(&owner);
        let wake = follow.rehooker();
        follow.product_hook(Arc::new(move |step, lanes| {
            let (Some(panels), Some(owner)) = (weak.upgrade(), product_owner.upgrade()) else { return };
            let epoch = step.epoch();
            let (stamp, session, push, due, resend) = {
                let mut panels = panels.lock().unwrap_or_else(PoisonError::into_inner);
                if panels.products.is_empty() || get(&owner).epoch() != epoch { return; }
                let Some(push) = panels.push.clone() else { return };
                let active = active.lock().unwrap_or_else(PoisonError::into_inner);
                let (clock, route, _, _) = panels.stamp;
                let stream = (epoch, active.seek, step.restep(), step.piece());
                let changed = panels.stream.replace(stream) != Some(stream) || step.reset();
                let resend = !changed && panels.resend.is_some_and(|reset| step.last() > reset);
                if changed || resend {
                    panels.resend = changed.then_some(step.last());
                    let Panels { due, products, .. } = &mut *panels;
                    due.clear();
                    due.extend(products.keys().map(|&id| (step.last(), id)));
                }
                let mut due = Vec::new();
                while let Some(id) = panel::product::take_due(&mut panels.due, step.last()) {
                    if let (Some(work), Some(token)) = (panels.products.get(&id), active.tokens.get(&id)) {
                        due.push((id, work.clone(), token.clone()));
                    }
                }
                ((clock, route, active.seek), panels.session, push, due, resend)
            };
            let mut replies = Vec::with_capacity(due.len());
            for (id, work, token) in due {
                if token.load(Ordering::Acquire) || get(&owner).epoch() != epoch { continue; }
                let Some(mut product) = work.take() else {
                    replies.push((id, token, step.last(), Some((panel::dated::WAITING, Vec::new()))));
                    continue;
                };
                if product.started_on != Some((epoch, stamp.2)) {
                    let hour = get(&owner).head(Instant::now()).min(step.last());
                    let task = Seed { hour, stamp, session, epoch, token: token.clone(), reads: active.clone(), push: push.clone(), wake: wake.clone() };
                    task.start(product, work, owner.clone(), get);
                    replies.push((id, token, step.last(), Some((panel::dated::WAITING, Vec::new()))));
                    continue;
                }
                product.resend |= resend;
                let input = panel::ProductStep { step, lanes, held: None, wake: Some(&wake) };
                let (next, change) = product.step(input, stamp, session, &token, &active, &push, get(&owner));
                work.put(product);
                replies.push((id, token, next, change));
            }
            let mut panels = panels.lock().unwrap_or_else(PoisonError::into_inner);
            let active = active.lock().unwrap_or_else(PoisonError::into_inner);
            if !active.current(epoch, stamp.2) || get(&owner).epoch() != epoch { return; }
            let mut batch = panel::dated::ProductBatch::new(stamp);
            for (id, token, next, change) in replies {
                if token.load(Ordering::Acquire) { continue; }
                panels.due.insert((next, id));
                if let Some((kind, body)) = change { batch.changed(id, step.last(), kind, &body); }
            }
            push.offer_from(session, PANEL, batch.finish(Some(step.last())));
        }));
        playhead.share_budget(crate::lead::budget())?;
        playhead.lead(crate::lead::LEAD);
        let driver_gate = Arc::new(WaitGate::new());
        let panel_gate = Arc::new(WaitGate::new());
        let panel_work = Arc::new(AtomicU64::new(0));
        follow.set_wake({
            let driver_gate = Arc::clone(&driver_gate);
            Arc::new(move || driver_gate.wake())
        });
        let driver_watch = playhead.watch({
            let driver_gate = Arc::clone(&driver_gate);
            Arc::new(move || driver_gate.wake())
        });
        let panel_watch = playhead.watch_control({
            let panel_gate = Arc::clone(&panel_gate);
            let panel_work = Arc::clone(&panel_work);
            Arc::new(move || {
                panel_work.fetch_add(1, Ordering::Release);
                panel_gate.wake();
            })
        });
        let (waiting, gate, follow_ref) = (panels.clone(), Arc::clone(&driver_gate), follow.clone());
        let driver = std::thread::Builder::new().name("follow driver".into()).spawn(move || {
            let mut taken = Vec::new();
            let mut next = Instant::now();
            while !stopping.load(Ordering::Acquire) {
                let seen = gate.stamp();
                let playhead = get(&driven);
                let playing = playhead.playing();
                let (has_work, told) = {
                    let held = waiting.lock().unwrap_or_else(PoisonError::into_inner);
                    (!held.products.is_empty() || !held.shown.is_empty() || follow_ref.has_work(), held.push.clone().map(|push| (push, held.stamp.1, held.stamp.3)))
                };
                let head = playhead.head(Instant::now());
                if let Some((push, route, seek)) = told { push.head(route, seek, head); }
                let release = if playing { head.saturating_add(playhead.step_hours().saturating_mul(crate::lead::RELEASE_FRAMES)) } else { head };
                let advance_ok = consumer.try_advance(release);
                let take_ok = consumer.try_take(release, &mut taken).is_some();
                taken.clear();
                let contended = !advance_ok || !take_ok;
                let timeout = if contended {
                    Some(Duration::from_millis(2))
                } else if playing && has_work {
                    let now = Instant::now();
                    if next <= now {
                        next = now + FRAME;
                    }
                    Some(next.saturating_duration_since(now))
                } else {
                    next = Instant::now();
                    None
                };
                if gate.wait(seen, timeout) {
                    return;
                }
            }
        })?;

        Ok(Arc::new(Self { owner, panels, reads, get, anchored, follow, reader, held, stop, driver_gate, panel_gate, panel_work, _driver_watch: driver_watch, _panel_watch: panel_watch, driver: Mutex::new(Some(driver)) }))
    }

    fn install_panel(&self, id: u32, panel: Box<dyn Panel>, lanes: u32, reader: Option<Box<dyn PanelReader>>) -> Result<(), String> {
        let class = (id % lanes.max(1), lanes.max(1));
        if let Some(exact) = panel.exact() {
            let mut panels = self.panels.lock().unwrap_or_else(PoisonError::into_inner);
            let push = panels.push.clone().ok_or("This playhead has no panel transport")?;
            let mut reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
            reads.register(id);
            panels.remove(id);
            let epoch = self.playhead().epoch();
            let hour = self.playhead().head(Instant::now());
            let host = panels.cells.clone();
            let product = StepProduct { id, pending_seed: None, started_on: None, read: exact.advance.clone(), lanes: Vec::new(), wire: Default::default(), last: None, joined: false, cells: Vec::new(), class, host, followed: None, input: None, resend: false };
            let task = Seed { hour, stamp: (panels.stamp.0, panels.stamp.1, reads.seek), session: panels.session, epoch, token: reads.tokens[&id].clone(), reads: self.reads.clone(), push, wake: self.follow.rehooker() };
            let work = Arc::new(Work::new(None));
            panels.due.insert((Hour::MIN, id));
            panels.products.insert(id, work.clone());
            drop(reads);
            drop(panels);
            task.start(product, work, self.owner.clone(), self.get);
            self.driver_gate.wake();
            return Ok(());
        }
        let reader = reader.expect("a prepared row reader");
        {
            let mut panels = self.panels.lock().unwrap_or_else(PoisonError::into_inner);
            self.reads.lock().unwrap_or_else(PoisonError::into_inner).register(id);
            panels.remove(id);
            panels.shown.insert(id, Arc::new(Work::new(Some(Shown::new(panel, reader, class)))));
            if panels.thread.is_none() {
                let (held, stopping, gate, work_seq) = (Arc::downgrade(&self.panels), Arc::clone(&self.stop), Arc::clone(&self.panel_gate), Arc::clone(&self.panel_work));
                let (owner, get, tokens) = (self.owner.clone(), self.get, self.reads.clone());
                let thread = std::thread::Builder::new()
                    .name("follow panels".into())
                    .spawn(move || {
                        let mut next: Option<Instant> = None;
                        let mut seen_work = u64::MAX;
                        while !stopping.load(Ordering::Acquire) {
                            let seen = gate.stamp();
                            let work = work_seq.load(Ordering::Acquire);
                            let now = Instant::now();
                            let Some(panels) = held.upgrade() else { return };
                            let playhead = get(&owner);
                            let playing = playhead.playing();
                            let poll = (playhead.frame_period() / 2).clamp(Duration::from_millis(4), PANEL_POLL);
                            let due = work != seen_work || next.is_some_and(|deadline| now >= deadline);
                            let active = if due {
                                let (jobs, stamp, version, session, push) = {
                                    let mut panels = panels.lock().unwrap_or_else(PoisonError::into_inner);
                                    panels.stamp.2 = playhead.epoch() as u32;
                                    let reads = tokens.lock().unwrap_or_else(PoisonError::into_inner);
                                    let jobs = panels.shown.iter().filter_map(|(&id, work)| Some((id, work.clone(), reads.tokens.get(&id)?.clone()))).collect::<Vec<_>>();
                                    (jobs, panels.stamp, panels.frame_version, panels.session, panels.push.clone())
                                };
                                let active = !jobs.is_empty() && playing;
                                if let Some(push) = push {
                                    for (id, work, token) in jobs {
                                        let Some(mut shown) = work.take() else { continue };
                                        let (push, tokens, gate, sequence, owner) = (push.clone(), tokens.clone(), gate.clone(), work_seq.clone(), owner.clone());
                                        run_panel(move || {
                                            if !token.load(Ordering::Acquire) { evaluate(id, &mut shown, stamp, version, &token, &tokens, &push, session, get(&owner)); }
                                            if work.put(shown) {
                                                sequence.fetch_add(1, Ordering::Release);
                                                gate.wake();
                                            }
                                        });
                                    }
                                }
                                seen_work = work;
                                active
                            } else {
                                !panels.lock().unwrap_or_else(PoisonError::into_inner).shown.is_empty() && playing
                            };
                            drop(panels);
                            let timeout = if active {
                                let deadline = match next {
                                    Some(deadline) if deadline > now => deadline,
                                    _ => {
                                        let deadline = now + poll;
                                        next = Some(deadline);
                                        deadline
                                    }
                                };
                                Some(deadline.saturating_duration_since(Instant::now()))
                            } else {
                                next = None;
                                None
                            };
                            if gate.wait(seen, timeout) {
                                return;
                            }
                        }
                    })
                    .map_err(|error| error.to_string())?;
                panels.thread = Some(thread);
            }
        }
        self.driver_gate.wake();
        self.wake_panels();
        Ok(())
    }

    pub fn playhead(&self) -> &Playhead {
        (self.get)(&self.owner)
    }

    fn wake_panels(&self) {
        self.panel_work.fetch_add(1, Ordering::Release);
        self.panel_gate.wake();
    }

    pub fn show_panels(&self, rows: Arc<dyn PanelRows>, push: Arc<Push>) {
        rows.on_publish({
            let panel_gate = Arc::clone(&self.panel_gate);
            let panel_work = Arc::clone(&self.panel_work);
            Arc::new(move || {
                panel_work.fetch_add(1, Ordering::Release);
                panel_gate.wake();
            })
        });
        {
            let mut panels = self.panels.lock().unwrap_or_else(PoisonError::into_inner);
            panels.rows = Some(rows);
            panels.push = Some(push);
        }
        self.driver_gate.wake();
        self.wake_panels();
    }
}

impl<E: Send + Sync + 'static> Hosted for PlayheadHost<E> {
    fn follow(&self) -> &Follow {
        &self.follow
    }

    fn lacks(&self, table: usize) -> Option<String> {
        lacking(table, self.held.get(table).copied().unwrap_or(false))
    }

    fn prepare_panel(&self, panel: Box<dyn Panel>) -> Result<crate::registry::PreparedPanel<'_>, String> {
        let reader = if panel.exact().is_some() {
            None
        } else {
            let rows = self.panels.lock().unwrap_or_else(PoisonError::into_inner).rows.clone().ok_or("This playhead's host gives panels no rows at the head")?;
            Some(rows.reader(panel.tables()).map_err(|error| format!("Rows at the head are refused: {error}"))?)
        };
        Ok(Box::new(move |id, lanes| self.install_panel(id, panel, lanes, reader)))
    }

    fn serve(&self, session: u64) {
        self.follow.serve(session);
        self.panels.lock().unwrap_or_else(PoisonError::into_inner).session = session;
        self.driver_gate.wake();
        self.wake_panels();
    }

    fn unpanel(&self, id: u32) {
        self.reads.lock().unwrap_or_else(PoisonError::into_inner).remove(id);
        self.panels.lock().unwrap_or_else(PoisonError::into_inner).remove(id);
        self.driver_gate.wake();
        self.wake_panels();
    }

    fn anchor(&self, anchor: Anchor) {
        let mut reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
        let anchor = Anchor { sought: anchor.sought || anchor.seek != reads.seek, ..anchor };
        self.follow.anchor(anchor.seek, || {
            (self.anchored)(&self.owner, anchor);
            self.playhead().epoch()
        });
        let epoch = self.playhead().epoch();
        reads.anchor(epoch, anchor.seek);
        drop(reads);
        {
            let mut panels = self.panels.lock().unwrap_or_else(PoisonError::into_inner);
            let epoch = epoch as u32;
            let previous = panels.pace.replace((anchor.playing, anchor.speed));
            panels.stamp.2 = epoch;
            let different_seek = panels.stamp.3 != anchor.seek;
            panels.stamp.3 = anchor.seek;
            if different_seek || anchor.sought || previous.is_some_and(|(playing, _)| playing && !anchor.playing) {
                panels.frame_version = panels.frame_version.wrapping_add(1);
                let mut reads = self.reads.lock().unwrap_or_else(PoisonError::into_inner);
                for id in panels.shown.keys() {
                    if let Some(token) = reads.tokens.get_mut(id) { std::mem::take(token).store(true, Ordering::Release); }
                }
            }
        }
        self.driver_gate.wake();
        self.wake_panels();
    }
}

impl<E: Send + Sync + 'static> Drop for PlayheadHost<E> {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.driver_gate.stop();
        self.panel_gate.stop();
        for token in self.reads.lock().unwrap_or_else(PoisonError::into_inner).tokens.values() { token.store(true, Ordering::Release); }
        if let Some(driver) = self.driver.lock().unwrap_or_else(PoisonError::into_inner).take() {
            let _ = driver.join();
        }
        let thread = self.panels.lock().unwrap_or_else(PoisonError::into_inner).thread.take();
        if let Some(thread) = thread {
            let _ = thread.join();
        }
        self.playhead().retire(self.reader);
    }
}

#[cfg(test)]
#[test]
fn a_retired_product_read_cannot_publish_its_seed() {
    let reads = Mutex::new(ProductReads::default());
    reads.lock().unwrap().register(4);
    let token = reads.lock().unwrap().tokens[&4].clone();
    let (started, wait_started) = std::sync::mpsc::channel();
    let (release, wait_release) = std::sync::mpsc::channel();
    let wait_release = Mutex::new(wait_release);
    let mut product = StepProduct {
        id: 4, lanes: Vec::new(), pending_seed: None, started_on: None, last: None, joined: false, wire: Default::default(), cells: Vec::new(), class: (0, 1), host: None, followed: None, input: None, resend: false,
        read: Arc::new(move |_, _, _, _| {
            started.send(()).unwrap();
            wait_release.lock().unwrap().recv().unwrap();
            panel::ProductRead { value: Arc::new(Ok(Value::Null)), rows: 0, fresh: true, pending: false, live: None, lanes: Vec::new() }
        }),
    };
    let push = Push::default();
    std::thread::scope(|threads| {
        threads.spawn(|| product.publish_seed((0, 1, 0), 0, 12, 1, &token, &reads, &push, || 1));
        wait_started.recv().unwrap();
        reads.lock().unwrap().anchor(2, 9);
        release.send(()).unwrap();
    });
    assert!(token.load(Ordering::Acquire));
    assert_eq!(push.counters().offered, 0);
    let mut current = reads.lock().unwrap();
    let new = current.tokens[&4].clone();
    current.remove(4);
    assert!(new.load(Ordering::Acquire));
}

#[cfg(test)]
#[test]
fn a_product_lane_redeclared_on_its_table_keeps_every_flag_it_asked_for() {
    let follow = Follow::new(0);
    let mut product = StepProduct {
        id: 4, lanes: Vec::new(), pending_seed: None, started_on: None, last: None, joined: false, wire: Default::default(), cells: Vec::new(), class: (0, 1),
        host: Some(HostCells { follow: follow.clone(), held: vec![true; 4] }), followed: None, input: None, resend: false,
        read: Arc::new(|_, _, _, _| panel::ProductRead { value: Arc::new(Ok(Value::Null)), rows: 0, fresh: true, pending: false, live: None, lanes: Vec::new() }),
    };
    let lane = |flag| Declared { table: 2, columns: vec![1], keyed: flag, passing: flag, hourly: flag };
    product.read_lanes(&[lane(false)]);
    product.read_lanes(&[lane(true)]);
    assert_eq!(product.lanes, [lane(true)]);
    assert_eq!(follow.declared().unwrap(), [lane(true)], "the playhead is told the lane's flags, not only its columns");
}

#[cfg(test)]
#[test]
fn wait_gate_never_misses_wake_before_wait_and_stops_blocked_waiter() {
    let gate = Arc::new(WaitGate::new());
    let seen = gate.stamp();
    gate.wake();
    assert!(!gate.wait(seen, None));

    let seen = gate.stamp();
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = Arc::clone(&gate);
    let handle = std::thread::spawn(move || {
        ready_tx.send(()).unwrap();
        let stopped = worker.wait(seen, None);
        done_tx.send(stopped).unwrap();
    });
    ready_rx.recv().unwrap();
    gate.stop();
    assert!(done_rx.recv().unwrap());
    handle.join().unwrap();
}
