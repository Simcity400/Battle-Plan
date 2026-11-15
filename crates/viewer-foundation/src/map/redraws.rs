//! What asked the map's overlay to lay out or draw again, what it made, and what it answered from
//! what it kept, counted per presented frame for the paced report ([`take`]).

use std::{cell::Cell, sync::Mutex};

static TALLY: Mutex<Vec<(&'static str, u64)>> = Mutex::new(Vec::new());

thread_local! {
    /// This thread runs work apart from any frame ([`apart`]).
    static APART: Cell<bool> = const { Cell::new(false) };
}

/// Runs `work` with what it counts kept out of the frames' tally: a worker's build, which lands in
/// whatever frame is current when it runs.
pub fn apart<T>(work: impl FnOnce() -> T) -> T {
    let was = APART.with(|apart| apart.replace(true));
    let value = work();
    APART.with(|apart| apart.set(was));
    value
}

/// Whether this thread runs work apart from any frame ([`apart`]): a worker's build.
pub fn apart_now() -> bool {
    APART.with(Cell::get)
}

/// Adds `count` to `name` for this frame, while frames are measured (`replay_frame::cpu`).
pub fn note(name: &'static str, count: u64) {
    if count == 0 || !replay_frame::cpu::enabled() || APART.with(Cell::get) {
        return;
    }
    let mut tally = TALLY.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    match tally.iter_mut().find(|(held, _)| *held == name) {
        Some((_, held)) => *held += count,
        None => tally.push((name, count)),
    }
}

/// Every name counted since the last call, with its count.
pub fn take() -> Vec<(&'static str, u64)> {
    std::mem::take(&mut *TALLY.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
}
