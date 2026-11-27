
use crate::{answer, invalid, panel};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::{self, Read, Write},
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    time::Instant,
};

pub const FOLLOW: u8 = 1;
pub const PANEL: u8 = 2;
pub const PART: usize = 1 << 22;
pub const OVERFLOW_REASON: &str = "This page fell too far behind while it was not responding.";
const HEAD: usize = 12;
type OverflowHook = Arc<dyn Fn(u64) + Send + Sync>;

pub fn write_frame(out: &mut impl Write, kind: u8, sequence: u32, payload: &[u8]) -> io::Result<()> {
    if payload.is_empty() {
        write_part(out, kind, false, sequence, payload)?;
    }
    let mut parts = payload.chunks(PART).peekable();
    while let Some(part) = parts.next() {
        write_part(out, kind, parts.peek().is_some(), sequence, part)?;
    }
    out.flush()
}

fn write_part(out: &mut impl Write, kind: u8, more: bool, sequence: u32, part: &[u8]) -> io::Result<()> {
    let mut head = [0u8; HEAD];
    head[..4].copy_from_slice(&(part.len() as u32).to_le_bytes());
    head[4] = kind;
    head[5] = u8::from(more);
    head[8..].copy_from_slice(&sequence.to_le_bytes());
    out.write_all(&head)?;
    out.write_all(part)
}

pub fn read_frame(input: &mut impl Read, payload: &mut Vec<u8>) -> io::Result<Option<(u8, u32)>> {
    payload.clear();
    let mut named = None;
    loop {
        let mut head = [0u8; HEAD];
        match input.read_exact(&mut head) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof && named.is_none() => return Ok(None),
            Err(error) => return Err(error),
        }
        let length = u32::from_le_bytes(head[..4].try_into().expect("four bytes")) as usize;
        if length > PART || head[5] > 1 || head[6..8] != [0, 0] {
            return Err(invalid("A pushed frame's head is not one"));
        }
        let part = (head[4], u32::from_le_bytes(head[8..].try_into().expect("four bytes")));
        if named.is_some_and(|named| named != part) {
            return Err(invalid("A pushed message's parts are of two messages"));
        }
        named = Some(part);
        let at = payload.len();
        payload.resize(at + length, 0);
        input.read_exact(&mut payload[at..])?;
        if head[5] == 0 {
            return Ok(named);
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PushCounters {
    pub offered: u64,
    pub written: u64,
    pub dropped: u64,
    pub unwritten: u64,
    pub most_queued: u64,
    pub bytes: u64,
    pub credited: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Panels {
    None,
    One(u32),
    Many(HashSet<u32>),
    All,
}

impl Panels {
    fn overlaps(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::None, _) | (_, Self::None) => false,
            (Self::All, _) | (_, Self::All) => true,
            (Self::One(a), Self::One(b)) => a == b,
            (Self::One(a), Self::Many(set)) | (Self::Many(set), Self::One(a)) => set.contains(a),
            (Self::Many(a), Self::Many(b)) => {
                let (small, big) = if a.len() <= b.len() { (a, b) } else { (b, a) };
                small.iter().any(|id| big.contains(id))
            }
        }
    }

    fn extend(&mut self, other: &Self) {
        match (&mut *self, other) {
            (Self::All, _) | (_, Self::None) => {}
            (_, Self::All) => *self = Self::All,
            (Self::None, _) => *self = other.clone(),
            (Self::One(a), Self::One(b)) if a == b => {}
            (Self::One(a), Self::One(b)) => *self = Self::Many(HashSet::from([*a, *b])),
            (Self::One(a), Self::Many(ids)) => { let mut set = ids.clone(); set.insert(*a); *self = Self::Many(set); }
            (Self::Many(set), Self::One(id)) => { set.insert(*id); }
            (Self::Many(set), Self::Many(ids)) => set.extend(ids.iter().copied()),
        }
    }

    #[cfg(debug_assertions)]
    fn covers(&self, bytes: &[u8]) -> bool {
        let Ok(records) = panel::dated::records(bytes) else { return true };
        records.iter().all(|r| match self {
            Self::All => true,
            Self::One(id) => r.kind != panel::dated::COMPLETED && *id == r.id,
            Self::Many(set) => r.kind != panel::dated::COMPLETED && set.contains(&r.id),
            Self::None => false,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Access {
    route: Option<u32>,
    panels: Panels,
    reads_followed: bool,
    writes_followed: bool,
}

impl Access {
    fn notice() -> Self { Self { route: None, panels: Panels::None, reads_followed: false, writes_followed: false } }
    fn empty(route: u32) -> Self { Self { route: Some(route), panels: Panels::None, reads_followed: false, writes_followed: false } }
    fn follow(route: u32, reset: bool) -> Self { Self { route: Some(route), panels: if reset { Panels::All } else { Panels::None }, reads_followed: true, writes_followed: true } }
    fn panel(route: u32, panels: Panels, registers_cells: bool) -> Self { Self { route: Some(route), panels, reads_followed: registers_cells, writes_followed: false } }

    fn commutes(&self, other: &Self) -> bool {
        let (Some(a), Some(b)) = (self.route, other.route) else { return true };
        a != b || (!((self.writes_followed && other.reads_followed) || (other.writes_followed && self.reads_followed)) && !self.panels.overlaps(&other.panels))
    }

    fn extend(&mut self, other: &Self) {
        self.panels.extend(&other.panels);
        self.reads_followed |= other.reads_followed;
        self.writes_followed |= other.writes_followed;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stream {
    Notice,
    Steps { route: u32, seek: u32, due: i32 },
    Panels { route: u32, seek: u32, due: i32 },
}

impl Stream {
    fn of(kind: u8, bytes: &[u8]) -> Self {
        if kind == FOLLOW && bytes.len() >= 24 && bytes.starts_with(&answer::MAGIC.to_le_bytes()) {
            let word = |at| u32::from_le_bytes(bytes[at..at + 4].try_into().expect("a dated header word"));
            let mut due = word(8) as i32;
            let mut at = 24;
            while let Some(header) = bytes.get(at..).and_then(|tail| tail.get(..16)) {
                due = due.max(i32::from_le_bytes(header[4..8].try_into().expect("answer hour")));
                let size = u32::from_le_bytes(header[8..12].try_into().expect("answer length"));
                at += 16 + size as usize;
            }
            Self::Steps { route: word(4) >> 1, seek: word(16), due }
        } else if kind == PANEL && let Some((route, seek)) = panel::dated::stamp(bytes) {
            let (mut due, mut waits) = (None::<i32>, i32::MIN);
            let mut at = 24;
            while let Some(header) = bytes.get(at..).and_then(|tail| tail.get(..16)) {
                let hour = i32::from_le_bytes(header[4..8].try_into().expect("panel hour"));
                if header[8] == panel::dated::WAITING {
                    waits = waits.max(hour);
                } else {
                    due = Some(due.map_or(hour, |due| due.max(hour)));
                }
                let size = u32::from_le_bytes(header[12..16].try_into().expect("panel length")) as usize;
                at += (16 + size).next_multiple_of(8);
            }
            Self::Panels { route, seek, due: due.unwrap_or(waits) }
        } else {
            Self::Notice
        }
    }

    fn version(self) -> Option<(u32, u32)> {
        match self {
            Self::Notice => None,
            Self::Steps { route, seek, .. } => Some((route, seek)),
            Self::Panels { route, seek, .. } => Some((route, seek)),
        }
    }
}

struct Queued {
    stream: Stream,
    access: Access,
    kind: u8,
    bytes: Vec<u8>,
    sequence: u32,
    offered: Instant,
    session: u64,
    terminal: bool,
}

struct State {
    charge: playhead::Charge,
    limit: usize,
    queue: VecDeque<Queued>,
    bytes: usize,
    sent: u64,
    released: u64,
    credit: usize,
    session: u64,
    heads: HashMap<u32, (u32, i32)>,
    admitted: HashMap<u32, u32>,
    next: u32,
    pressed: Option<u64>,
    overflowed: HashSet<u32>,
    on_overflow: HashMap<u32, OverflowHook>,
    stopped: bool,
    failed: Option<String>,
    counters: PushCounters,
}

#[derive(Default)]
pub struct Push {
    state: Mutex<State>,
    ready: Condvar,
}

impl Default for State {
    fn default() -> Self { Self::new(crate::lead::budget()) }
}

fn lock(held: &Mutex<State>) -> MutexGuard<'_, State> {
    held.lock().unwrap_or_else(PoisonError::into_inner)
}

impl State {
    fn new(budget: Arc<playhead::ByteBudget>) -> Self {
        let limit = budget.limit();
        Self { charge: budget.charge(), limit, queue: VecDeque::new(), bytes: 0, sent: 0, released: 0, credit: limit / 2, session: 0, heads: HashMap::new(), admitted: HashMap::new(), next: 0, pressed: None, overflowed: HashSet::new(), on_overflow: HashMap::new(), stopped: false, failed: None, counters: PushCounters::default() }
    }

    fn reached(&self, stream: Stream) -> bool {
        match stream {
            Stream::Steps { route, seek, due } | Stream::Panels { route, seek, due } => self.heads.get(&route).is_some_and(|&(at, head)| at == seek && due <= head),
            Stream::Notice => false,
        }
    }

    fn in_flight(&self) -> usize {
        usize::try_from(self.sent - self.released).unwrap_or(usize::MAX)
    }

    fn charged(&mut self) {
        self.charge.resize(self.bytes + self.in_flight());
    }

    fn sendable(&self) -> Option<usize> {
        let in_flight = self.in_flight();
        let mut blocked: HashMap<u32, Access> = HashMap::new();
        for (at, queued) in self.queue.iter().enumerate() {
            let waits = !queued.terminal && queued.access.route.is_some_and(|route| blocked.get(&route).is_some_and(|held| !held.commutes(&queued.access)));
            if queued.terminal || (!waits && (queued.stream == Stream::Notice || in_flight == 0 || in_flight + answer::charge(&queued.bytes) <= self.credit || (self.reached(queued.stream) && (!self.charge.full() || self.pressed.is_none_or(|at| self.released > at))))) {
                return Some(at);
            }
            if let Some(route) = queued.access.route {
                blocked.entry(route).or_insert_with(|| Access::empty(route)).extend(&queued.access);
            }
        }
        None
    }
}

impl Push {
    pub fn with_budget(budget: Arc<playhead::ByteBudget>) -> Self {
        Self { state: Mutex::new(State::new(budget)), ready: Condvar::new() }
    }

    pub fn watch_overflow(&self, route: u32, hook: OverflowHook) {
        lock(&self.state).on_overflow.insert(route, hook);
    }

    pub fn unwatch_overflow(&self, route: u32) {
        lock(&self.state).on_overflow.remove(&route);
    }

    pub fn offer(&self, bytes: Vec<u8>) {
        self.offer_follow(self.session(), bytes);
    }

    pub fn offer_follow(&self, session: u64, bytes: Vec<u8>) {
        let stream = Stream::of(FOLLOW, &bytes);
        let access = match stream {
            Stream::Steps { route, .. } => Access::follow(route, answer::resets(&bytes)),
            Stream::Notice | Stream::Panels { .. } => Access::notice(),
        };
        self.enqueue(session, FOLLOW, bytes, stream, access);
    }

    pub fn offer_panel(&self, session: u64, bytes: Vec<u8>, panels: Panels, registers_cells: bool) {
        let stream = Stream::of(PANEL, &bytes);
        let access = match stream {
            Stream::Panels { route, .. } => {
                #[cfg(debug_assertions)]
                assert!(panels.covers(&bytes), "a panel message's stated access must cover every record it carries");
                Access::panel(route, panels, registers_cells)
            }
            Stream::Notice | Stream::Steps { .. } => Access::notice(),
        };
        self.enqueue(session, PANEL, bytes, stream, access);
    }

    pub fn offer_terminal(&self, session: u64, kind: u8, bytes: Vec<u8>) {
        let stream = Stream::of(kind, &bytes);
        let mut state = lock(&self.state);
        if state.stopped || state.failed.is_some() || session != state.session {
            return;
        }
        if let Some((route, stamp)) = stream.version() {
            let latest = *state.admitted.entry(route).or_insert(stamp);
            if latest.wrapping_sub(stamp) as i32 > 0 {
                return;
            }
            if latest != stamp {
                state.admitted.insert(route, stamp);
                let mut removed = 0;
                state.queue.retain(|q| {
                    let keep = q.stream.version().is_none_or(|(r, old)| r != route || stamp.wrapping_sub(old) as i32 <= 0);
                    if !keep { removed += answer::charge(&q.bytes); }
                    keep
                });
                state.bytes -= removed;
            }
        }
        let sequence = state.next;
        state.next = state.next.wrapping_add(1);
        state.counters.offered += 1;
        let charge = answer::charge(&bytes);
        state.bytes += charge;
        state.queue.push_back(Queued { stream, access: Access::notice(), kind, bytes, sequence, offered: Instant::now(), session, terminal: true });
        state.charged();
        state.counters.most_queued = state.counters.most_queued.max(state.queue.len() as u64);
        drop(state);
        self.ready.notify_one();
    }

    fn enqueue(&self, session: u64, kind: u8, bytes: Vec<u8>, stream: Stream, access: Access) {
        let overflow = {
            let mut state = lock(&self.state);
            if state.stopped || state.failed.is_some() || session != state.session {
                return;
            }
            let sequence = state.next;
            state.next = state.next.wrapping_add(1);
            state.counters.offered += 1;
            let charge = answer::charge(&bytes);
            if let Some((route, stamp)) = stream.version() {
                let latest = *state.admitted.entry(route).or_insert(stamp);
                if latest.wrapping_sub(stamp) as i32 > 0 {
                    return;
                }
                if latest != stamp {
                    state.admitted.insert(route, stamp);
                    let mut removed = 0;
                    state.queue.retain(|q| {
                        let keep = q.stream.version().is_none_or(|(r, old)| r != route || stamp.wrapping_sub(old) as i32 <= 0);
                        if !keep { removed += answer::charge(&q.bytes); }
                        keep
                    });
                    state.bytes -= removed;
                }
                if state.overflowed.contains(&route) {
                    state.counters.dropped += 1;
                    crate::lead::count_dropped(1);
                    state.charged();
                    return;
                }
                let route_queued: usize = state.queue.iter().filter(|q| !q.terminal && q.access.route == Some(route)).map(|q| answer::charge(&q.bytes)).sum();
                if route_queued > 0 && route_queued + charge > state.limit {
                    let (mut dropped, mut removed) = (1u64, 0usize);
                    state.queue.retain(|q| if !q.terminal && q.access.route == Some(route) { removed += answer::charge(&q.bytes); dropped += 1; false } else { true });
                    state.bytes -= removed;
                    state.overflowed.insert(route);
                    state.counters.dropped += dropped;
                    crate::lead::count_dropped(dropped);
                    state.charged();
                    state.on_overflow.get(&route).cloned().map(|hook| (hook, state.session))
                } else {
                    state.bytes += charge;
                    let session = state.session;
                    state.queue.push_back(Queued { stream, access, kind, bytes, sequence, offered: Instant::now(), session, terminal: false });
                    state.charged();
                    state.counters.most_queued = state.counters.most_queued.max(state.queue.len() as u64);
                    drop(state);
                    self.ready.notify_one();
                    return;
                }
            } else {
                state.bytes += charge;
                let session = state.session;
                state.queue.push_back(Queued { stream, access, kind, bytes, sequence, offered: Instant::now(), session, terminal: false });
                state.charged();
                state.counters.most_queued = state.counters.most_queued.max(state.queue.len() as u64);
                drop(state);
                self.ready.notify_one();
                return;
            }
        };
        self.ready.notify_all();
        if let Some((hook, session)) = overflow {
            hook(session);
        }
    }

    pub fn take(&self) -> Option<(Vec<u8>, u32, Instant)> {
        self.take_kind().map(|(_, bytes, sequence, offered)| (bytes, sequence, offered))
    }

    pub fn credit(&self, session: u64, released: u64) {
        let mut state = lock(&self.state);
        if session != state.session { return; }
        let released = released.min(state.sent);
        if released <= state.released { return; }
        state.counters.credited += released - state.released;
        state.released = released;
        state.charged();
        drop(state);
        self.ready.notify_all();
    }

    pub fn restart(&self) -> u64 {
        let mut state = lock(&self.state);
        state.session += 1;
        state.sent = 0;
        state.released = 0;
        state.pressed = None;
        state.overflowed.clear();
        state.counters.unwritten += state.queue.len() as u64;
        state.queue.clear();
        state.bytes = 0;
        state.failed = None;
        state.heads.clear();
        state.admitted.clear();
        let session = state.session;
        state.charged();
        drop(state);
        self.ready.notify_all();
        session
    }

    pub fn take_sent(&self) -> Option<(u8, Vec<u8>, u64)> {
        let mut state = lock(&self.state);
        loop {
            if let Some(held) = Self::send(&mut state) {
                return Some((held.kind, held.bytes, held.session));
            }
            if state.stopped {
                return None;
            }
            state = self.ready.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
    }

    pub fn head(&self, route: u32, seek: u32, hour: i32) {
        let mut state = lock(&self.state);
        if state.heads.insert(route, (seek, hour)) == Some((seek, hour)) { return; }
        drop(state);
        self.ready.notify_all();
    }

    pub fn in_flight(&self) -> usize {
        lock(&self.state).in_flight()
    }

    pub fn session(&self) -> u64 {
        lock(&self.state).session
    }

    pub fn try_take(&self) -> Option<(u8, Vec<u8>, u32, Instant)> {
        Self::send(&mut lock(&self.state)).map(|held| (held.kind, held.bytes, held.sequence, held.offered))
    }

    fn send(state: &mut State) -> Option<Queued> {
        let at = state.sendable()?;
        let held = state.queue.remove(at)?;
        let charge = answer::charge(&held.bytes);
        let pressed = !held.terminal && held.stream != Stream::Notice && state.charge.full() && state.in_flight() != 0 && state.in_flight() + charge > state.credit;
        state.bytes -= charge;
        if pressed { state.pressed = Some(state.released); }
        if held.stream != Stream::Notice { state.sent += charge as u64; }
        state.charged();
        state.counters.written += 1;
        state.counters.bytes += held.bytes.len() as u64;
        Some(held)
    }

    pub fn take_kind(&self) -> Option<(u8, Vec<u8>, u32, Instant)> {
        let mut state = lock(&self.state);
        loop {
            if let Some(held) = Self::send(&mut state) {
                return Some((held.kind, held.bytes, held.sequence, held.offered));
            }
            if state.stopped {
                return None;
            }
            state = self.ready.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
    }

    pub fn take_waiting(&self) -> Vec<Vec<u8>> {
        let mut state = lock(&self.state);
        state.bytes = 0;
        state.charged();
        state.queue.drain(..).map(|held| held.bytes).collect()
    }

    pub fn stop(&self) {
        lock(&self.state).stopped = true;
        self.ready.notify_all();
    }

    pub fn counters(&self) -> PushCounters {
        lock(&self.state).counters
    }

    pub fn failed(&self) -> Option<String> {
        lock(&self.state).failed.clone()
    }

    pub fn drain(&self, out: &mut impl Write) -> io::Result<()> {
        while let Some((kind, bytes, sequence, _)) = self.take_kind() {
            if let Err(error) = write_frame(out, kind, sequence, &bytes) {
                let mut state = lock(&self.state);
                state.failed = Some(format!("Pushed answers stopped: their pipe failed ({error})"));
                state.stopped = true;
                state.counters.written -= 1;
                state.counters.bytes -= bytes.len() as u64;
                state.counters.unwritten += 1 + state.queue.len() as u64;
                state.queue.clear();
                state.bytes = 0;
                state.released = state.sent;
                state.charge.resize(0);
                return Err(error);
            }
        }
        Ok(())
    }
}
