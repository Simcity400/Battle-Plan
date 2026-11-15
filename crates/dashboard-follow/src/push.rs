//! Pushed messages from the stepper to a pipe: frames of `length u32, kind u8, more u8, two zero
//! bytes, sequence u32` and the payload, little-endian: followed answers ([`FOLLOW`]) and panels
//! ([`PANEL`]) share the pipe. A message of any size goes as frames of at most [`PART`] bytes, each
//! but its last marked `more`, which the reader joins. The stepper and the panels hand each message
//! to [`Push`], a queue of [`QUEUED`] messages that never blocks: past it the oldest is merged into
//! the next of its kind, the newer answer or data of each id kept, so a slow reader makes older ones
//! superseded, never a longer queue; two that do not merge are both sent as they are. A writer
//! thread drains it; a write that fails stops it and names why ([`Push::failed`], [`failure`]).

use crate::{answer, invalid, panel};
use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    sync::{Condvar, Mutex, MutexGuard, PoisonError},
    time::Instant,
};

/// A frame of followed answers ([`crate::Message`]'s bytes).
pub const FOLLOW: u8 = 1;
/// A frame of panels ([`panel::message`]).
pub const PANEL: u8 = 2;
/// Messages waiting, past which they merge.
pub const QUEUED: usize = 4;
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
    /// Merges that failed: both messages are sent as they are.
    pub unmerged: u64,
    /// Messages not written because the pipe failed ([`Push::failed`]).
    pub unwritten: u64,
    pub most_queued: u64,
    pub bytes: u64,
}

/// Whether `newer` is what `older` merges into: of its kind, and a followed message of its follow.
fn merges(older: &Queued, newer: &Queued) -> bool {
    older.kind == newer.kind && (older.kind == PANEL || answer::route(&older.bytes) == answer::route(&newer.bytes))
}

struct Queued {
    kind: u8,
    bytes: Vec<u8>,
    sequence: u32,
    offered: Instant,
    /// It did not merge into a later one: it is sent as it is.
    kept: bool,
}

#[derive(Default)]
struct State {
    queue: VecDeque<Queued>,
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

fn lock(held: &Mutex<State>) -> MutexGuard<'_, State> {
    held.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Push {
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
        state.queue.push_back(Queued { kind, bytes, sequence, offered: Instant::now(), kept: false });
        while state.queue.len() > QUEUED {
            // The oldest message not kept with a later one of its kind (and follow) is merged into it.
            let queue = &state.queue;
            let pair = (0..queue.len()).filter(|&at| !queue[at].kept).find_map(|at| (at + 1..queue.len()).find(|&later| merges(&queue[at], &queue[later])).map(|later| (at, later)));
            let Some((at, later)) = pair else { break };
            let (older, newer) = (&queue[at], &queue[later]);
            let merged = if older.kind == PANEL { panel::merge(&older.bytes, &newer.bytes) } else { answer::merge(&older.bytes, &newer.bytes) };
            match merged {
                Ok((merged, superseded)) => {
                    let older = state.queue.remove(at).expect("queued");
                    let newer = &mut state.queue[later - 1];
                    newer.bytes = merged;
                    newer.offered = older.offered;
                    state.counters.superseded += u64::from(superseded);
                    state.counters.merges += 1;
                }
                Err(_) => {
                    state.queue[at].kept = true;
                    state.counters.unmerged += 1;
                }
            }
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
                return Err(error);
            }
        }
        Ok(())
    }
}
