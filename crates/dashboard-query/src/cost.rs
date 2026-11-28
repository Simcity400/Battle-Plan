
use replay_recording::columnar_frames::Recording;
use replay_data::rows::{TABLE_COUNT, TableId};
use std::{cell::Cell, io, sync::{Arc, LazyLock}, time::Instant};

thread_local! {
    static STORAGE: Cell<u64> = const { Cell::new(0) };
    static DEPTH: Cell<u32> = const { Cell::new(0) };
    #[cfg(test)]
    static CALLS: Cell<u64> = const { Cell::new(0) };
}

struct StorageRead {
    depth: u32,
    started: Option<Instant>,
}

impl Drop for StorageRead {
    fn drop(&mut self) {
        if let Some(started) = self.started {
            STORAGE.set(STORAGE.get().wrapping_add(started.elapsed().as_nanos() as u64));
        }
        DEPTH.set(self.depth);
    }
}

fn storage_read() -> StorageRead {
    let depth = DEPTH.get();
    DEPTH.set(depth + 1);
    #[cfg(test)]
    if depth == 0 {
        CALLS.set(CALLS.get().wrapping_add(1));
    }
    StorageRead { depth, started: (depth == 0).then(Instant::now) }
}

pub fn storage<T>(read: impl FnOnce() -> T) -> T {
    let _read = storage_read();
    read()
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

pub fn add(nanos: u64) {
    STORAGE.set(STORAGE.get().wrapping_add(nanos));
}

pub fn measure<T>(work: impl FnOnce() -> T) -> (T, u64) {
    struct Restore(u64);
    impl Drop for Restore {
        fn drop(&mut self) {
            STORAGE.set(STORAGE.get().wrapping_add(self.0));
        }
    }
    let _restore = Restore(STORAGE.replace(0));
    let value = work();
    let spent = STORAGE.replace(0);
    (value, spent)
}

#[cfg(test)]
pub fn take_calls() -> u64 {
    CALLS.replace(0)
}

pub fn lifetimes(recording: &Recording, run: u32, table: TableId) -> io::Result<Vec<(u128, i64, i64)>> {
    declared(table)?;
    storage(|| replay_lanes::lanes::lifetimes(recording, run, table))
}

pub fn recorded_lifetimes(replay: &replay::campaign::Replay, run: u32, table: TableId) -> io::Result<Arc<Vec<(u128, i64, i64)>>> {
    declared(table)?;
    replay.row_lifetimes(run, table, storage_read)
}

pub fn presences(recording: &Recording, run: u32, table: TableId, keys: (u128, u128), from: i64, to: i64) -> io::Result<Vec<(u128, Vec<(i64, i64)>)>> {
    declared(table)?;
    storage(|| replay_lanes::lanes::presences(recording, run, table, keys, from, to))
}

pub fn declared(table: TableId) -> io::Result<()> {
    static DECLARED: LazyLock<io::Result<[bool; TABLE_COUNT]>> = LazyLock::new(|| {
        let mut tables = [false; TABLE_COUNT];
        for (table, _) in dashboard_declare::summaries()? {
            tables[table] = true;
        }
        Ok(tables)
    });
    if DECLARED.as_ref().map_err(|error| io::Error::new(error.kind(), error.to_string()))?[table.index()] {
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
