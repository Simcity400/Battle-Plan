
use crate::{answer, invalid, panel};
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    sync::{Condvar, Mutex, MutexGuard, PoisonError},
    time::Instant,
};

pub const FOLLOW: u8 = 1;
pub const PANEL: u8 = 2;
pub const PART: usize = 1 << 22;
const HEAD: usize = 12;
pub const FAILURE: u32 = u32::from_le_bytes(*b"FEN1");

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

pub fn failure(reason: &str) -> Vec<u8> {
    [&FAILURE.to_le_bytes()[..], reason.as_bytes()].concat()
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PushCounters {
    pub offered: u64,
    pub written: u64,
    pub superseded: u64,
    pub merges: u64,
    pub pressure_merges: u64,
    pub unmerged: u64,
    pub unwritten: u64,
    pub most_queued: u64,
    pub bytes: u64,
    pub credited: u64,
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
            let mut due = i32::MIN;
            let mut at = 24;
            while let Some(header) = bytes.get(at..).and_then(|tail| tail.get(..16)) {
                due = due.max(i32::from_le_bytes(header[4..8].try_into().expect("panel hour")));
                let size = u32::from_le_bytes(header[12..16].try_into().expect("panel length")) as usize;
                at += (16 + size).next_multiple_of(8);
            }
            Self::Panels { route, seek, due }
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
    kind: u8,
    bytes: Vec<u8>,
    sequence: u32,
    offered: Instant,
    kept: bool,
    session: u64,
}

struct State {
    charge: playhead::Charge,
    queue: VecDeque<Queued>,
    bytes: usize,
    sent: u64,
    released: u64,
    credit: usize,
    session: u64,
    heads: std::collections::HashMap<u32, (u32, i32)>,
    admitted: std::collections::HashMap<u32, u32>,
    next: u32,
    stopped: bool,
    failed: Option<String>,
    counters: PushCounters,
}

#[derive(Default)]
pub struct Push {
    state: Mutex<State>,
    ready: Condvar,
}

impl Push {
    pub fn with_budget(budget: std::sync::Arc<playhead::ByteBudget>) -> Self {
        Self {
            state: Mutex::new(State::new(budget)),
            ready: Condvar::new(),
        }
    }
}

impl Default for State {
    fn default() -> Self { Self::new(crate::lead::budget()) }
}

fn lock(held: &Mutex<State>) -> MutexGuard<'_, State> {
    held.lock().unwrap_or_else(PoisonError::into_inner)
}

impl State {
    fn new(budget: std::sync::Arc<playhead::ByteBudget>) -> Self {
        Self { charge: budget.charge(), queue: VecDeque::new(), bytes: 0, sent: 0, released: 0, credit: budget.limit() / 2, session: 0, heads: std::collections::HashMap::new(), admitted: std::collections::HashMap::new(), next: 0, stopped: false, failed: None, counters: PushCounters::default() }
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

    fn charged(&mut self) -> io::Result<()> {
        self.charge.resize(self.bytes + self.in_flight())
    }

    fn sendable(&self) -> Option<usize> {
        let mut blocked: Vec<u32> = Vec::new();
        for (at, queued) in self.queue.iter().enumerate() {
            let route = queued.stream.version().map(|(route, _)| route);
            if route.is_some_and(|route| blocked.contains(&route)) { continue; }
            if queued.stream == Stream::Notice || self.in_flight() == 0 || self.in_flight() + answer::charge(&queued.bytes) <= self.credit || self.reached(queued.stream) {
                return Some(at);
            }
            if let Some(route) = route { blocked.push(route); }
        }
        None
    }

    fn settle(&mut self) {
        if let Err(error) = self.charged() {
            let sequence = self.next;
            self.next = self.next.wrapping_add(1);
            self.overflow(&error.to_string(), sequence);
        }
    }

    fn overflow(&mut self, reason: &str, sequence: u32) {
        self.released = self.sent;
        let _ = self.charge.resize(0);
        let bytes = failure(reason);
        self.counters.unwritten += self.queue.len() as u64;
        self.queue.clear();
        self.bytes = bytes.len();
        self.queue.push_back(Queued { stream: Stream::Notice, kind: FOLLOW, bytes, sequence, offered: Instant::now(), kept: false, session: self.session });
        self.failed = Some(reason.to_owned());
    }

    fn merge_passed(&mut self) {
        let mut at = 0;
        while at < self.queue.len() {
            let older = &self.queue[at];
            let Some(version) = older.stream.version() else {
                at += 1;
                continue;
            };
            if older.kept || !self.reached(older.stream) {
                at += 1;
                continue;
            }
            let kind = older.kind;
            let Some(later) = (at + 1..self.queue.len()).find(|&i| {
                let q = &self.queue[i];
                q.kind == kind && q.stream.version() == Some(version)
            }) else {
                at += 1;
                continue;
            };
            if self.reached(self.queue[later].stream) && self.merge(at, later) {
                continue;
            }
            at += 1;
        }
    }

    fn refuse_largest_panel(&mut self, reason: &str) -> bool {
        let replacement = panel::dated::refused_record_len(reason) as i64;
        let mut savings: std::collections::BTreeMap<u32, i64> = std::collections::BTreeMap::new();
        for queued in &self.queue {
            if !matches!(queued.stream, Stream::Panels { .. }) {
                continue;
            }
            let Ok(records) = panel::dated::records(&queued.bytes) else {
                continue;
            };
            for r in records {
                if r.kind == panel::dated::DATA {
                    *savings.entry(r.id).or_default() += r.byte_len() as i64 - replacement;
                }
            }
        }
        let Some((offender, _)) = savings
            .into_iter()
            .filter(|&(_, saved)| saved > 0)
            .max_by_key(|&(id, saved)| (saved, std::cmp::Reverse(id)))
        else {
            return false;
        };
        let mut reduced = false;
        for queued in &mut self.queue {
            if !matches!(queued.stream, Stream::Panels { .. }) {
                continue;
            }
            let Ok(Some(rebuilt)) = panel::dated::refuse_panel(&queued.bytes, offender, reason) else {
                continue;
            };
            let before = answer::charge(&queued.bytes);
            let after = answer::charge(&rebuilt);
            self.bytes = self.bytes - before + after;
            queued.bytes = rebuilt;
            reduced = true;
        }
        reduced
    }

    fn merge(&mut self, at: usize, later: usize) -> bool {
        let (older, newer) = (&self.queue[at], &self.queue[later]);
        let result = match older.stream {
            Stream::Panels { .. } => panel::dated::merge(&older.bytes, &newer.bytes),
            Stream::Notice => return false,
            Stream::Steps { .. } => answer::merge(&older.bytes, &newer.bytes),
        };
        match result {
            Ok((bytes, superseded)) => {
                let older = self.queue.remove(at).expect("queued");
                let newer = &mut self.queue[later - 1];
                self.bytes -= answer::charge(&older.bytes) + answer::charge(&newer.bytes);
                self.bytes += answer::charge(&bytes);
                newer.stream = Stream::of(newer.kind, &bytes);
                newer.bytes = bytes;
                newer.offered = older.offered;
                self.counters.superseded += u64::from(superseded);
                self.counters.merges += 1;
                true
            }
            Err(_) => {
                self.queue[at].kept = true;
                self.counters.unmerged += 1;
                false
            }
        }
    }

    fn compact(&mut self, stream: Stream) {
        let Some(version) = stream.version() else { return };
        let last = self.queue.len() - 1;
        let kind = self.queue[last].kind;
        if matches!(stream, Stream::Steps { .. })
            && let Some(at) = (0..last).rev().find(|&i| self.queue[i].kind == kind && self.queue[i].stream.version() == Some(version))
            && !self.queue[at].kept && self.queue[at].stream == stream {
            self.merge(at, last);
        }
        self.merge_passed();
        while self.charged().is_err() {
            let pair = {
                let mut ours = self.queue.iter().enumerate()
                    .filter(|(_, q)| q.kind == kind && q.stream.version() == Some(version) && !q.kept && self.reached(q.stream)).map(|(at, _)| at);
                (ours.next(), ours.next())
            };
            let (Some(at), Some(later)) = pair else { break };
            if !self.merge(at, later) { break; }
            self.counters.pressure_merges += 1;
            crate::lead::count_merge();
        }
    }
}

impl Push {
    pub fn with_budget(budget: std::sync::Arc<playhead::ByteBudget>) -> Self {
        Self { state: Mutex::new(State::new(budget)), ready: Condvar::new() }
    }

    pub fn offer(&self, bytes: Vec<u8>) {
        self.offer_kind(FOLLOW, bytes);
    }

    pub fn offer_kind(&self, kind: u8, bytes: Vec<u8>) {
        let session = self.session();
        self.offer_from(session, kind, bytes);
    }

    pub fn offer_from(&self, session: u64, kind: u8, bytes: Vec<u8>) {
        let mut state = lock(&self.state);
        if state.stopped || state.failed.is_some() || session != state.session {
            return;
        }
        let sequence = state.next;
        state.next = state.next.wrapping_add(1);
        state.counters.offered += 1;
        let stream = Stream::of(kind, &bytes);
        if let Some((route, stamp)) = stream.version() {
            let latest = *state.admitted.entry(route).or_insert(stamp);
            if latest.wrapping_sub(stamp) as i32 > 0 {
                return;
            }
            state.admitted.insert(route, stamp);
            let mut removed = 0;
            state.queue.retain(|q| {
                let keep = q.stream.version().is_none_or(|(r, old)| r != route || stamp.wrapping_sub(old) as i32 <= 0);
                if !keep { removed += answer::charge(&q.bytes); }
                keep
            });
            state.bytes -= removed;
        }
        state.bytes += answer::charge(&bytes);
        let session = state.session;
        state.queue.push_back(Queued { stream, kind, bytes, sequence, offered: Instant::now(), kept: false, session });
        state.compact(stream);
        while let Err(error) = state.charged() {
            let reason = error.to_string();
            if !state.refuse_largest_panel(&reason) {
                state.overflow(&reason, sequence);
                break;
            }
        }
        state.counters.most_queued = state.counters.most_queued.max(state.queue.len() as u64);
        drop(state);
        self.ready.notify_one();
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
        state.settle();
        drop(state);
        self.ready.notify_all();
    }

    pub fn restart(&self) -> u64 {
        let mut state = lock(&self.state);
        state.session += 1;
        state.sent = 0;
        state.released = 0;
        state.counters.unwritten += state.queue.len() as u64;
        state.queue.clear();
        state.bytes = 0;
        state.failed = None;
        state.heads.clear();
        state.admitted.clear();
        let session = state.session;
        state.settle();
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
        state.merge_passed();
        if state.failed.is_none() {
            let _ = state.charged();
        }
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
        state.bytes -= charge;
        if held.stream != Stream::Notice { state.sent += charge as u64; }
        state.settle();
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
        state.settle();
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
                let _ = state.charge.resize(0);
                return Err(error);
            }
        }
        Ok(())
    }
}
