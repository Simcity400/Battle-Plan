
use replay_recording::columnar_frames::Recording;
use replay_data::rows::TableId;
use std::{cell::Cell, io, time::Instant};

thread_local! {
    static STORAGE: Cell<u64> = const { Cell::new(0) };
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

pub fn storage<T>(read: impl FnOnce() -> T) -> T {
    let depth = DEPTH.get();
    DEPTH.set(depth + 1);
    let started = (depth == 0).then(Instant::now);
    let out = read();
    if let Some(started) = started {
        STORAGE.set(STORAGE.get().wrapping_add(started.elapsed().as_nanos() as u64));
    }
    DEPTH.set(depth);
    out
}

pub fn outside<T>(work: impl FnOnce() -> T) -> T {
    let depth = DEPTH.get();
    if depth == 0 {
        return work();
    }
    DEPTH.set(0);
    let started = Instant::now();
    let out = work();
    let spent = started.elapsed().as_nanos() as u64;
    DEPTH.set(depth);
    STORAGE.set(STORAGE.get().wrapping_sub(spent));
    out
}

pub fn take() -> u64 {
    STORAGE.replace(0)
}

pub fn lifetimes(recording: &Recording, run: u32, table: TableId) -> io::Result<Vec<(u128, i64, i64)>> {
    declared(table)?;
    storage(|| replay_lanes::lanes::lifetimes(recording, run, table))
}

pub fn presences(recording: &Recording, run: u32, table: TableId, keys: (u128, u128), from: i64, to: i64) -> io::Result<Vec<(u128, Vec<(i64, i64)>)>> {
    declared(table)?;
    storage(|| replay_lanes::lanes::presences(recording, run, table, keys, from, to))
}

pub fn declared(table: TableId) -> io::Result<()> {
    if dashboard_declare::summaries()?.iter().any(|(held, _)| *held == table.index()) {
        return Ok(());
    }
    Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{table:?} keeps no lifetimes: no series, log or lifetimes declaration summarizes it")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifetimes_of_a_table_no_declaration_summarizes_are_refused_never_read_as_no_key() {
        assert!(declared(TableId::NavalBattles).is_ok(), "the naval battle log finds its battles by their lives");
        assert!(declared(TableId::NavalResults).is_ok(), "and their retained results");
        assert_eq!(declared(TableId::SunkShips).unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }
}
