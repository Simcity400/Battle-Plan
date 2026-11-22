use std::{
    io,
    sync::{Condvar, Mutex, MutexGuard, PoisonError},
};

pub const CLOSED: &str = "BattlePlan is closing, so nothing more is written.";

#[derive(Default)]
struct Count {
    running: usize,
    closed: bool,
}

pub struct Writes {
    count: Mutex<Count>,
    idle: Condvar,
}

pub struct Writing<'a>(&'a Writes);

impl Drop for Writing<'_> {
    fn drop(&mut self) {
        self.0.lock().running -= 1;
        self.0.idle.notify_all();
    }
}

impl Default for Writes {
    fn default() -> Self {
        Self::new()
    }
}

impl Writes {
    pub const fn new() -> Self {
        Self { count: Mutex::new(Count { running: 0, closed: false }), idle: Condvar::new() }
    }

    fn lock(&self) -> MutexGuard<'_, Count> {
        self.count.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn begin(&self) -> io::Result<Writing<'_>> {
        let mut count = self.lock();
        if count.closed {
            return Err(io::Error::new(io::ErrorKind::Interrupted, CLOSED));
        }
        count.running += 1;
        Ok(Writing(self))
    }

    pub fn close(&self) {
        let mut count = self.lock();
        count.closed = true;
        while count.running > 0 {
            count = self.idle.wait(count).unwrap_or_else(PoisonError::into_inner);
        }
    }
}

pub static WRITES: Writes = Writes::new();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_waits_for_the_writes_under_way_and_refuses_new_ones() {
        let writes = Writes::new();
        let writing = writes.begin().unwrap();
        std::thread::scope(|scope| {
            let (closing, closed) = std::sync::mpsc::channel();
            let writes = &writes;
            scope.spawn(move || {
                writes.close();
                closing.send(()).unwrap();
            });
            while !writes.lock().closed {
                std::thread::yield_now();
            }
            assert!(closed.try_recv().is_err(), "the write under way holds the exit");
            assert_eq!(writes.begin().err().map(|error| error.to_string()), Some(CLOSED.to_owned()), "no write starts once BattlePlan closes");
            drop(writing);
            closed.recv().unwrap();
        });
        writes.close();
    }
}
