
use std::{cell::Cell, sync::Mutex};

static TALLY: Mutex<Vec<(&'static str, u64)>> = Mutex::new(Vec::new());

thread_local! {
    static APART: Cell<bool> = const { Cell::new(false) };
}

pub fn apart<T>(work: impl FnOnce() -> T) -> T {
    let was = APART.with(|apart| apart.replace(true));
    let value = work();
    APART.with(|apart| apart.set(was));
    value
}

pub fn apart_now() -> bool {
    APART.with(Cell::get)
}

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

pub fn take() -> Vec<(&'static str, u64)> {
    std::mem::take(&mut *TALLY.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
}
