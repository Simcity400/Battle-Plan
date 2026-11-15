//! Each table's conversion time on a thread that armed the clock: the spool
//! and its helpers while they convert packs. Elsewhere [`timed`] only runs the
//! conversion. A table converted inside another's timing counts toward the outer.

use replay_data::rows::{TABLE_COUNT, TableId};
use std::cell::{Cell, RefCell};

/// 0: disarmed; 1: armed, between tables; 2: inside a timed table.
const DISARMED: u8 = 0;
const ARMED: u8 = 1;
const TIMING: u8 = 2;

thread_local! {
    static STATE: Cell<u8> = const { Cell::new(DISARMED) };
    static TABLE_NS: RefCell<[u64; TABLE_COUNT]> = const { RefCell::new([0; TABLE_COUNT]) };
}

/// Runs `convert`, charging its time to `table` when this thread is armed and
/// no other table's timing encloses it.
#[inline]
pub fn timed<R>(table: TableId, convert: impl FnOnce() -> R) -> R {
    if STATE.get() != ARMED {
        return convert();
    }
    STATE.set(TIMING);
    let started = std::time::Instant::now();
    let output = convert();
    let ns = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
    STATE.set(ARMED);
    TABLE_NS.with_borrow_mut(|tables| {
        let slot = &mut tables[table.index()];
        *slot = slot.saturating_add(ns);
    });
    output
}

/// This thread's clock, armed until dropped.
#[must_use = "the clock is disarmed when dropped"]
pub struct Armed(std::marker::PhantomData<*const ()>);

/// Arms this thread's clock with every table at zero.
pub fn arm() -> Armed {
    TABLE_NS.with_borrow_mut(|tables| *tables = [0; TABLE_COUNT]);
    STATE.set(ARMED);
    Armed(std::marker::PhantomData)
}

/// Adds the tables this thread timed since arming or the last drain into
/// `into`, and returns their sum.
pub fn drain(into: &mut [u64; TABLE_COUNT]) -> u64 {
    TABLE_NS.with_borrow_mut(|tables| {
        let mut sum = 0u64;
        for (total, ns) in into.iter_mut().zip(tables.iter_mut()) {
            let ns = std::mem::take(ns);
            *total = total.saturating_add(ns);
            sum = sum.saturating_add(ns);
        }
        sum
    })
}

impl Drop for Armed {
    fn drop(&mut self) {
        STATE.set(DISARMED);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_armed_outermost_tables_are_charged() {
        let mut tables = [0; TABLE_COUNT];
        timed(TableId::Clock, std::thread::yield_now);
        let clock = arm();
        timed(TableId::Countries, || {
            timed(TableId::Provinces, std::thread::yield_now);
            std::thread::yield_now();
        });
        let sum = drain(&mut tables);
        drop(clock);
        timed(TableId::States, std::thread::yield_now);
        assert_eq!(tables[TableId::Clock.index()], 0);
        assert_eq!(tables[TableId::Provinces.index()], 0);
        assert_eq!(tables[TableId::States.index()], 0);
        assert_eq!(sum, tables[TableId::Countries.index()]);
        let _clock = arm();
        assert_eq!(drain(&mut [0; TABLE_COUNT]), 0);
    }
}
