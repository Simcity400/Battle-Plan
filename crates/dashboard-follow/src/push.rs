//! Pushed messages from the stepper to a pipe: frames of `length u32, kind u8, more u8, two zero
//! bytes, sequence u32` and the payload, little-endian: followed answers ([`FOLLOW`]) and panels
//! ([`PANEL`]) share the pipe. A message of any size goes as frames of at most [`PART`] bytes, each
//! but its last marked `more`. A writer drains the dated queue and reports pipe failures by name.

use crate::{answer, invalid, panel};
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    sync::{Condvar, Mutex, MutexGuard, PoisonError},
    time::Instant,
};

/// A frame of followed answers ([`crate::Message`]'s bytes).
pub const FOLLOW: u8 = 1;
/// A frame of panels ([`panel::dated::message`]).
pub const PANEL: u8 = 2;
/// The most payload one frame carries: a longer message goes in parts.
pub const PART: usize = 1 << 22;
const HEAD: usize = 12;
/// `FEN1`: the first bytes of [`failure`]'s payload.
pub const FAILURE: u32 = u32::from_le_bytes(*b"FEN1");

/// `payload` as one message of frames.
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

/// The next message, its parts joined: kind, sequence and payload (into `payload`); none at the
/// pipe's end.
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

/// What the pipe's reader hands the widgets when pushed answers stop: [`FAILURE`] and `reason`.
pub fn failure(reason: &str) -> Vec<u8> {
    [&FAILURE.to_le_bytes()[..], reason.as_bytes()].concat()
}

/// Counted since the queue was made.
#[derive(Clone, Copy, Debug, Default)]
pub struct PushCounters {
    pub offered: u64,
    pub written: u64,
    /// Answers dropped because a newer one of the same id was queued behind them.
    pub superseded: u64,
    pub merges: u64,
    pub pressure_merges: u64,
    /// Merges that failed: both messages are sent as they are.
    pub unmerged: u64,
    /// Messages not written because the pipe failed ([`Push::failed`]).
    pub unwritten: u64,
    pub most_queued: u64,
    pub bytes: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stream {
    Notice,
    Steps { route: u32, seek: u32, due: i32 },
    Panels { route: u32, seek: u32 },
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
            Self::Panels { route, seek }
        } else {
            Self::Notice
        }
    }

    fn version(self) -> Option<(u32, u32)> {
        match self {
            Self::Notice => None,
            Self::Steps { route, seek, .. } => Some((route, seek)),
            Self::Panels { route, seek } => Some((route, seek)),
        }
    }
}

struct Queued {
    stream: Stream,
    kind: u8,
    bytes: Vec<u8>,
    sequence: u32,
    offered: Instant,
    /// It did not merge into a later one: it is sent as it is.
    kept: bool,
}

struct State {
    charge: playhead::Charge,
    queue: VecDeque<Queued>,
    bytes: usize,
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

impl Default for State {
    fn default() -> Self { Self::new(crate::lead::budget()) }
}

fn lock(held: &Mutex<State>) -> MutexGuard<'_, State> {
    held.lock().unwrap_or_else(PoisonError::into_inner)
}

impl State {
    fn new(budget: std::sync::Arc<playhead::ByteBudget>) -> Self {
        Self { charge: budget.charge(), queue: VecDeque::new(), bytes: 0, next: 0, stopped: false, failed: None, counters: PushCounters::default() }
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
                self.bytes -= older.bytes.len() + newer.bytes.len();
                self.bytes += bytes.len();
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
        while self.charge.resize(self.bytes).is_err() {
            let mut ours = self.queue.iter().enumerate().filter(|(_, q)| q.kind == kind && q.stream.version() == Some(version));
            let (Some((at, first)), Some((later, _))) = (ours.next(), ours.next()) else { break };
            if first.kept || !self.merge(at, later) { break; }
            self.counters.pressure_merges += 1;
            crate::lead::count_merge();
        }
    }
}

impl Push {
    pub fn with_budget(budget: std::sync::Arc<playhead::ByteBudget>) -> Self {
        Self { state: Mutex::new(State::new(budget)), ready: Condvar::new() }
    }

    /// Queues a message of followed answers; never waits.
    pub fn offer(&self, bytes: Vec<u8>) {
        self.offer_kind(FOLLOW, bytes);
    }

    /// Queues a message of `kind`; never waits.
    pub fn offer_kind(&self, kind: u8, bytes: Vec<u8>) {
        let mut state = lock(&self.state);
        if state.stopped {
            return;
        }
        let sequence = state.next;
        state.next = state.next.wrapping_add(1);
        state.counters.offered += 1;
        let stream = Stream::of(kind, &bytes);
        if let Some((route, stamp)) = stream.version() {
            if state.queue.iter().any(|q| q.kind == kind && q.stream.version().is_some_and(|(r, held)| r == route && held.wrapping_sub(stamp) as i32 > 0)) {
                return;
            }
            let mut removed = 0;
            state.queue.retain(|q| {
                let keep = q.kind != kind || q.stream.version().is_none_or(|(r, old)| r != route || stamp.wrapping_sub(old) as i32 <= 0);
                if !keep { removed += q.bytes.len(); }
                keep
            });
            state.bytes -= removed;
        }
        state.bytes += bytes.len();
        state.queue.push_back(Queued { stream, kind, bytes, sequence, offered: Instant::now(), kept: false });
        state.compact(stream);
        let queued = state.bytes;
        if let Err(error) = state.charge.resize(queued) {
            let reason = error.to_string();
            state.charge.resize(0).expect("releasing the failed queue");
            let bytes = failure(&reason);
            state.counters.unwritten += state.queue.len() as u64;
            state.queue.clear();
            state.bytes = bytes.len();
            state.queue.push_back(Queued { stream: Stream::Notice, kind: FOLLOW, bytes, sequence, offered: Instant::now(), kept: false });
            state.failed = Some(reason);
            state.stopped = true;
        }
        state.counters.most_queued = state.counters.most_queued.max(state.queue.len() as u64);
        drop(state);
        self.ready.notify_one();
    }

    /// The oldest message, its sequence and when it was offered; waits for one, none once stopped.
    pub fn take(&self) -> Option<(Vec<u8>, u32, Instant)> {
        self.take_kind().map(|(_, bytes, sequence, offered)| (bytes, sequence, offered))
    }

    /// [`Self::take`] with the message's kind.
    pub fn take_kind(&self) -> Option<(u8, Vec<u8>, u32, Instant)> {
        let mut state = lock(&self.state);
        loop {
            if let Some(held) = state.queue.pop_front() {
                state.bytes -= held.bytes.len();
                let remaining = state.bytes;
                state.charge.resize(remaining).expect("releasing a sent packet");
                state.counters.written += 1;
                state.counters.bytes += held.bytes.len() as u64;
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
        state.charge.resize(0).expect("releasing queued packets");
        state.queue.drain(..).map(|held| held.bytes).collect()
    }

    pub fn stop(&self) {
        lock(&self.state).stopped = true;
        self.ready.notify_all();
    }

    pub fn counters(&self) -> PushCounters {
        lock(&self.state).counters
    }

    /// Why the pipe failed, once a write to it has.
    pub fn failed(&self) -> Option<String> {
        lock(&self.state).failed.clone()
    }

    /// Writes every message to `out` as frames until stopped or the pipe fails; a failure stops
    /// the queue and is kept by name ([`Self::failed`]).
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
                state.charge.resize(0).expect("releasing queued packets");
                return Err(error);
            }
        }
        Ok(())
    }
}
