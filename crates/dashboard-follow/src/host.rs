
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
        Arc, Condvar, Mutex, PoisonError,
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
    cells: Vec<(Group, u32)>,
    class: (u32, u32),
}

impl Shown {
    fn new(panel: Box<dyn Panel>, reader: Box<dyn PanelReader>, class: (u32, u32)) -> Self {
        Self { panel, reader, frame: None, data: None, pushed: None, cells: Vec::new(), class }
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

#[derive(Default)]
struct Panels {
    stamp: (i32, u32, u32, u32),
    products: BTreeMap<u32, StepProduct>,
    due: BTreeSet<(Hour, u32)>,
    stream: Option<(u64, u32, u64, Option<usize>)>,
    resend: Option<Hour>,
    pace: Option<(bool, f64)>,
    rows: Option<Arc<dyn PanelRows>>,
    push: Option<Arc<Push>>,
    shown: BTreeMap<u32, Shown>,
    thread: Option<JoinHandle<()>>,
    cells: Option<HostCells>,
    session: u64,
}

#[derive(Default)]
struct ProductReads {
    epoch: u64,
    seek: u32,
    tokens: BTreeMap<u32, Arc<AtomicBool>>,
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
        if let Some(token) = self.tokens.remove(&id) { token.store(true, Ordering::Release); }
    }

    fn register(&mut self, id: u32) {
        self.remove(id);
        self.tokens.insert(id, Arc::default());
    }
}

struct StepProduct {
    id: u32,
    seed: Option<(u64, Hour)>,
    read: panel::ProductReader,
    lanes: Vec<Declared>,
    wire: panel::product::Delivery,
    last: Option<(u64, u32, Hour)>,
    joined: bool,
    next: Option<Hour>,
    cells: Vec<(Group, u32)>,
    class: (u32, u32),
    host: Option<HostCells>,
    followed: Option<Arc<PanelData>>,
    input: Option<Arc<PanelData>>,
    resend: bool,
}

impl StepProduct {
    fn publish_seed(&mut self, stamp: (i32, u32, u32), session: u64, id: u32, epoch: u64, token: &AtomicBool,
        reads: &Mutex<ProductReads>, push: &Push, current_epoch: impl Fn() -> u64) {
        let Some((registered, head)) = self.seed.take() else { return };
        if epoch != registered || token.load(Ordering::Acquire) { return; }
        let (clock, route, seek) = stamp;
        let change = self.read_at(seek, epoch, head, true, token, None).flatten();
        let reads = reads.lock().unwrap_or_else(PoisonError::into_inner);
        if reads.current(epoch, seek) && !token.load(Ordering::Acquire) && current_epoch() == epoch
            && let Some((kind, body)) = change {
            push.offer_from(session, PANEL, panel::dated::encoded(clock, route, seek, id, head, kind, body));
            self.wire.commit();
        }
    }

    fn read_at(&mut self, page_seek: u32, epoch: u64, hour: Hour, reset: bool, cancelled: &AtomicBool, step: Option<panel::ProductStep<'_>>) -> Option<Option<(u8, &[u8])>> {
        let seed = reset || self.last.is_none_or(|(e, g, h)| e != epoch || g != page_seek || h > hour);
        let send = seed || std::mem::take(&mut self.resend);
        let held = self.host.as_ref().map(|host| host.held.clone());
        let step = step.map(|step| panel::ProductStep { held: held.as_deref(), ..step });
        let read = catch_unwind(AssertUnwindSafe(|| (self.read)(hour, send, cancelled, step)))
            .unwrap_or_else(|p| panel::ProductRead { value: Arc::new(Err(format!("Panel read at hour {hour} failed: {}", panicked(&*p)))),
                rows: 0, fresh: true, pending: false, live: None, lanes: Vec::new() });
        self.read_lanes(&read.lanes);
        if read.pending { return None; }
        self.last = Some((epoch, page_seek, hour));
        self.next = hour.checked_add(1);
        let value = self.follow_cells(&read.value, read.live.as_deref(), hour, send, read.fresh);
        Some(self.wire.update(send, &value))
    }

    fn read_lanes(&mut self, read: &[Declared]) {
        let Some(host) = &self.host else { return };
        let mut lanes = self.lanes.clone();
        for wanted in read.iter().filter(|wanted| host.held.get(wanted.table) == Some(&true)) {
            match lanes.iter_mut().find(|lane| lane.table == wanted.table) {
                Some(lane) if wanted.columns.iter().all(|column| lane.columns.contains(column)) && (!wanted.keyed || lane.keyed) && (!wanted.hourly || lane.hourly) => {}
                Some(lane) => {
                    lane.columns.extend(&wanted.columns);
                    lane.columns.sort_unstable();
                    lane.columns.dedup();
                    lane.keyed |= wanted.keyed;
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
        if let Some(mut product) = self.products.remove(&id) {
            if let Some(hour) = product.next { self.due.remove(&(hour, id)); }
            release(&mut product.cells, product.host.as_ref());
            if let Some(cells) = &self.cells { let _ = cells.follow.product_lanes(id, Vec::new()); }
        }
        if let Some(mut shown) = self.shown.remove(&id) {
            release(&mut shown.cells, self.cells.as_ref());
        }
    }
}

fn panicked(payload: &(dyn std::any::Any + Send)) -> String {
    payload.downcast_ref::<&str>().map(|text| (*text).to_owned()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_else(|| "a panic with no message".to_owned())
}

fn evaluate(panels: &mut Panels) {
    let (clock, route, epoch, page_seek) = panels.stamp;
    let session = panels.session;
    use panel::dated::{self, Update};
    let Some(push) = &panels.push else { return };
    for (&id, shown) in &mut panels.shown {
        if let Some((hour, reason)) = shown.reader.stopped() {
            let data = Err(reason);
            if shown.data.as_ref() != Some(&data) {
                release(&mut shown.cells, panels.cells.as_ref());
                push.offer_from(session, PANEL, dated::message(clock, route, page_seek, &[Update::Data(id, hour, &data)]));
                shown.data = Some(data);
            }
            continue;
        }
        let Some(frame) = shown.reader.frame() else { continue };
        let Some((through, frame_epoch)) = shown.reader.through_seek(&frame) else { continue };
        if frame_epoch as u32 != epoch { continue; }
        let same = shown.frame.as_ref().is_some_and(|f| Arc::ptr_eq(f, &frame));
        let data;
        let mut updates = Vec::new();
        if !same && shown.data.is_some() && shown.panel.unchanged(&frame) {
            shown.frame = Some(Arc::clone(&frame));
        } else if !same {
            let made = catch_unwind(AssertUnwindSafe(|| shown.panel.data(&frame)))
                .unwrap_or_else(|p| Err(format!("This panel could not be made at hour {} ({})", frame.hour, panicked(&*p))));
            data = with_cells(&frame, &made, shown.panel.live(), &mut shown.cells, panels.cells.as_ref(), shown.class);
            updates.push(Update::Data(id, frame.hour, &data));
            shown.frame = Some(Arc::clone(&frame));
            shown.data = Some(made);
            shown.pushed = Some(frame.hour);
        }
        if shown.pushed.is_some_and(|last| through > last) {
            updates.push(Update::Stands(id, through));
            shown.pushed = Some(through);
        }
        if !updates.is_empty() { push.offer_from(session, PANEL, dated::message(clock, route, page_seek, &updates)); }
    }
}

pub struct PlayheadHost<E: Send + Sync + 'static> {
    owner: Arc<E>,
    panels: Arc<Mutex<Panels>>,
    reads: Arc<Mutex<ProductReads>>,
    needs_seed: Arc<AtomicBool>,
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
        let reads = Arc::new(Mutex::new(ProductReads::default()));
        let active = reads.clone();
        let product_owner = Arc::downgrade(&owner);
        let wake = follow.rehooker();
        follow.product_hook(Arc::new(move |step, lanes| {
            let (Some(panels), Some(owner)) = (weak.upgrade(), product_owner.upgrade()) else { return };
            let mut panels = panels.lock().unwrap_or_else(PoisonError::into_inner);
            if panels.products.is_empty() || get(&owner).epoch() != step.epoch() { return; }
            let (clock, route, _, _) = panels.stamp;
            let session = panels.session;
            let page_seek = active.lock().unwrap_or_else(PoisonError::into_inner).seek;
            let Some(push) = panels.push.clone() else { return };
            let stamp = (clock, route, page_seek);
            let quantum = get(&owner).step_hours();
            let mut batch = panel::dated::ProductBatch::new(stamp);
            let mut delivered = Vec::new();
            let mut held_over = Vec::new();
            let mut pending = Vec::new();
            let stream = (step.epoch(), page_seek, step.restep(), step.piece());
            if panels.stream.replace(stream) != Some(stream) || step.reset() {
                let Panels { due, products, resend, .. } = &mut *panels;
                due.clear();
                due.extend(products.keys().map(|&id| (step.last(), id)));
                *resend = Some(step.last());
            } else if panels.resend.is_some_and(|reset| step.last() > reset) {
                panels.resend = None;
                let Panels { due, products, .. } = &mut *panels;
                due.clear();
                due.extend(products.keys().map(|&id| (step.last(), id)));
                for product in products.values_mut() { product.resend = true; }
            }
            while let Some(id) = panel::product::take_due(&mut panels.due, step.last()) {
                let Some(product) = panels.products.get_mut(&id) else { continue };
                let Some(cancelled) = active.lock().unwrap_or_else(PoisonError::into_inner).tokens.get(&id).cloned() else { continue };
                let current = || !cancelled.load(Ordering::Acquire) && get(&owner).epoch() == step.epoch();
                if !current() { held_over.push(id); continue; }
                if !step.reset() && product.seed.is_some_and(|(_, head)| head < step.last()) {
                    product.publish_seed(stamp, session, id, step.epoch(), &cancelled, &active, &push, || get(&owner).epoch());
                } else {
                    product.seed = None;
                }
                if !product.joined && let Some((epoch, g, mut h)) = product.last && (epoch, g) == (step.epoch(), page_seek) && !step.reset() {
                    while h.saturating_add(quantum) < step.first() {
                        h += quantum;
                        let (kind, body) = product.read_at(page_seek, epoch, h, false, &cancelled, None).flatten().unwrap_or((panel::dated::STANDS, &[]));
                        let reads = active.lock().unwrap_or_else(PoisonError::into_inner);
                        if !reads.current(epoch, page_seek) || !current() { break; }
                        push.offer_from(session, PANEL, panel::dated::encoded(clock, route, page_seek, id, h, kind, body));
                        product.wire.commit();
                    }
                }
                if !current() { held_over.push(id); continue; }
                let joining = !product.joined;
                let Some(change) = product.read_at(page_seek, step.epoch(), step.last(), step.reset(), &cancelled, Some(panel::ProductStep { step, lanes, held: None, wake: Some(&wake) })) else {
                    pending.push(id);
                    continue;
                };
                if !current() { held_over.push(id); continue; }
                delivered.push(id);
                if let Some((kind, body)) = change {
                    batch.changed(id, step.last(), kind, body);
                } else if joining {
                    batch.changed(id, step.last(), panel::dated::STANDS, &[]);
                }
                product.joined = true;
                if let Some(next) = product.next { panels.due.insert((next, id)); }
            }
            for id in held_over {
                if let Some(product) = panels.products.get_mut(&id) { product.resend = true; }
                panels.due.insert((step.last().saturating_add(1), id));
            }
            for &id in &pending { batch.changed(id, step.last(), panel::dated::WAITING, &[]); }
            for id in pending {
                if let Some(product) = panels.products.get_mut(&id) { product.resend = true; }
                panels.due.insert((step.last(), id));
            }
            let reads = active.lock().unwrap_or_else(PoisonError::into_inner);
            if reads.current(step.epoch(), page_seek) && get(&owner).epoch() == step.epoch() {
                push.offer_from(session, PANEL, batch.finish(Some(step.last())));
                for id in delivered { if let Some(product) = panels.products.get_mut(&id) { product.wire.commit(); } }
            } else {
                for id in delivered {
                    if let Some(product) = panels.products.get_mut(&id) { product.resend = true; }
                    panels.due.insert((step.last().saturating_add(1), id));
                }
            }
        }));
        playhead.share_budget(crate::lead::budget())?;
        playhead.lead(crate::lead::LEAD);
        let needs_seed = Arc::new(AtomicBool::new(false));
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
        let (seeds, waiting, tokens, gate, follow_ref) = (needs_seed.clone(), panels.clone(), reads.clone(), Arc::clone(&driver_gate), follow.clone());
        let driver = std::thread::Builder::new().name("follow driver".into()).spawn(move || {
            let mut taken = Vec::new();
            let mut next = Instant::now();
            while !stopping.load(Ordering::Acquire) {
                let seen = gate.stamp();
                let playhead = get(&driven);
                if seeds.swap(false, Ordering::AcqRel) {
                    let mut held = waiting.lock().unwrap_or_else(PoisonError::into_inner);
                    let (clock, route, _, _) = held.stamp;
                    let session = held.session;
                    if let Some(push) = held.push.clone() {
                        for (&id, product) in &mut held.products {
                            if product.seed.is_none() { continue; }
                            let reads = tokens.lock().unwrap_or_else(PoisonError::into_inner);
                            let Some(token) = reads.tokens.get(&id).cloned() else { continue };
                            let seek = reads.seek;
                            drop(reads);
                            product.publish_seed((clock, route, seek), session, id, playhead.epoch(), &token, &tokens, &push, || playhead.epoch());
                        }
                    }
                }
                let playing = playhead.playing();
                let (has_work, told) = {
                    let held = waiting.lock().unwrap_or_else(PoisonError::into_inner);
                    (!held.products.is_empty() || !held.shown.is_empty() || follow_ref.has_work(), held.push.clone().map(|push| (push, held.stamp.1, held.stamp.3)))
                };
                let head = playhead.head(Instant::now());
                if let Some((push, route, seek)) = told { push.head(route, seek, head); }
                let advance_ok = consumer.try_advance(head);
                let take_ok = consumer.try_take(head, &mut taken).is_some();
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

        Ok(Arc::new(Self { owner, panels, reads, needs_seed, get, anchored, follow, reader, held, stop, driver_gate, panel_gate, panel_work, _driver_watch: driver_watch, _panel_watch: panel_watch, driver: Mutex::new(Some(driver)) }))
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

    fn panel(&self, id: u32, panel: Box<dyn Panel>, lanes: u32) -> Result<(), String> {
        let class = (id % lanes.max(1), lanes.max(1));
        {
            let mut panels = self.panels.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(exact) = panel.exact() {
                let read = exact.advance.clone();
                if panels.push.is_none() { return Err("This playhead has no panel transport".into()); }
                panels.remove(id);
                self.reads.lock().unwrap_or_else(PoisonError::into_inner).register(id);
                let seed = Some((self.playhead().epoch(), self.playhead().head(Instant::now())));
                let host = Some(HostCells { follow: self.follow.clone(), held: self.held.clone() });
                let product = StepProduct { id, seed, read, lanes: Vec::new(), wire: Default::default(), last: None, joined: false, next: Some(Hour::MIN), cells: Vec::new(), class, host, followed: None, input: None, resend: false };
                panels.due.insert((Hour::MIN, id));
                panels.products.insert(id, product);
                self.needs_seed.store(true, Ordering::Release);
                drop(panels);
                self.driver_gate.wake();
                return Ok(());
            }
            let rows = panels.rows.clone().ok_or("This playhead's host gives panels no rows at the head")?;
            let reader = rows.reader(panel.tables()).map_err(|error| format!("Rows at the head are refused: {error}"))?;
            panels.remove(id);
            panels.shown.insert(id, Shown::new(panel, reader, class));
            if panels.thread.is_none() {
                let (held, stopping, gate, work_seq) = (Arc::downgrade(&self.panels), Arc::clone(&self.stop), Arc::clone(&self.panel_gate), Arc::clone(&self.panel_work));
                let (owner, get) = (self.owner.clone(), self.get);
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
                                let mut panels = panels.lock().unwrap_or_else(PoisonError::into_inner);
                                panels.stamp.2 = playhead.epoch() as u32;
                                evaluate(&mut panels);
                                seen_work = work;
                                !panels.shown.is_empty() && playing
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
        self.follow.anchor(anchor.seek, || {
            (self.anchored)(&self.owner, anchor);
            self.playhead().epoch()
        });
        let epoch_u64 = self.playhead().epoch();
        reads.anchor(epoch_u64, anchor.seek);
        drop(reads);
        {
            let mut panels = self.panels.lock().unwrap_or_else(PoisonError::into_inner);
            let epoch = epoch_u64 as u32;
            let previous = panels.pace.replace((anchor.playing, anchor.speed));
            panels.stamp.2 = epoch;
            let different_seek = panels.stamp.3 != anchor.seek;
            panels.stamp.3 = anchor.seek;
            if different_seek || anchor.sought || previous.is_some_and(|(playing, _)| playing && !anchor.playing) {
                for shown in panels.shown.values_mut() {
                    shown.frame = None;
                    shown.data = None;
                    shown.pushed = None;
                }
            }
            let head = self.playhead().head(Instant::now());
            let mut reseed = false;
            for product in panels.products.values_mut() {
                if product.seed.is_some() || (!anchor.playing && (different_seek || anchor.sought || !product.joined)) {
                    product.seed = Some((epoch_u64, head));
                    reseed = true;
                }
            }
            if reseed {
                self.needs_seed.store(true, Ordering::Release);
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
        id: 4, lanes: Vec::new(), seed: Some((1, 12)), last: None, next: None, joined: false, wire: Default::default(), cells: Vec::new(), class: (0, 1), host: None, followed: None, input: None, resend: false,
        read: Arc::new(move |_, _, _, _| {
            started.send(()).unwrap();
            wait_release.lock().unwrap().recv().unwrap();
            panel::ProductRead { value: Arc::new(Ok(Value::Null)), rows: 0, fresh: true, pending: false, live: None, lanes: Vec::new() }
        }),
    };
    let push = Push::default();
    std::thread::scope(|threads| {
        threads.spawn(|| product.publish_seed((0, 1, 0), 0, 4, 1, &token, &reads, &push, || 1));
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
