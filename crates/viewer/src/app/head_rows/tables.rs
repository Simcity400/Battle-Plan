//! The open map's overlay tables, made from rows at the head off the frame thread: one build in
//! flight, under discovery on the thread that runs it. A build that read a table its rows do not
//! hold makes nothing: the table is learned, the tables laid out last stay, and the overlay is
//! loading by name until a build over rows holding it is made.

use replay_data::rows::TableId;
use std::sync::{Arc, Mutex, PoisonError};

/// What a build gave: the value, or the tables it read that its rows did not hold.
#[derive(Debug)]
pub(crate) enum Outcome<T> {
    Made(T),
    Missed(Vec<TableId>),
}

/// Hands a build to a worker; false while the workers are busy (asked again next frame).
pub(crate) type Submit<'a> = &'a dyn Fn(Box<dyn FnOnce() + Send>) -> bool;

type Slot<K, T> = Arc<Mutex<Option<(K, Outcome<T>)>>>;

#[derive(Debug)]
pub(crate) struct Job<K, T> {
    /// The build in flight and where it puts what it gives.
    running: Option<(K, Slot<K, T>)>,
    /// The last key built or missed: asked again only for another.
    done: Option<K>,
    /// The tables the last build missed, named while they load.
    loading: Vec<TableId>,
}

impl<K, T> Default for Job<K, T> {
    fn default() -> Self {
        Self { running: None, done: None, loading: Vec::new() }
    }
}

impl<K: Clone + PartialEq + Send + 'static, T: Send + 'static> Job<K, T> {
    /// Starts `make` for `key` unless it is the key last built or missed, or a build is in flight.
    pub(crate) fn ask(&mut self, key: &K, make: impl FnOnce() -> T + Send + 'static, submit: Submit<'_>, wake: impl Fn() + Send + 'static) {
        if self.done.as_ref() == Some(key) || self.running.is_some() {
            return;
        }
        let slot: Slot<K, T> = Arc::default();
        let (out, made) = (Arc::clone(&slot), key.clone());
        let work = move || {
            let (value, missed) = replay_frame::frame::discovering(make);
            let outcome = if missed.is_empty() { Outcome::Made(value) } else { Outcome::Missed(missed) };
            *out.lock().unwrap_or_else(PoisonError::into_inner) = Some((made, outcome));
            wake();
        };
        if !submit(Box::new(work)) {
            return;
        }
        self.running = Some((key.clone(), slot));
    }

    /// What the build in flight gave, once it has: the value made, or the tables it missed, which
    /// stay named loading until a later build is made.
    pub(crate) fn take(&mut self) -> Option<(K, Outcome<T>)> {
        let given = self.running.as_ref()?.1.lock().unwrap_or_else(PoisonError::into_inner).take()?;
        self.running = None;
        self.done = Some(given.0.clone());
        match &given.1 {
            Outcome::Made(_) => self.loading.clear(),
            Outcome::Missed(missed) => self.loading.clone_from(missed),
        }
        Some(given)
    }

    /// Whether a build is in flight.
    pub(crate) fn running(&self) -> bool {
        self.running.is_some()
    }

    /// The tables the last build missed, while they load.
    pub(crate) fn loading(&self) -> &[TableId] {
        &self.loading
    }
}

#[cfg(test)]
mod tests {
    use super::{Job, Outcome};

    #[test]
    fn a_build_that_misses_a_table_makes_nothing_names_it_and_is_not_asked_again_for_the_same_rows() {
        let here = |work: Box<dyn FnOnce() + Send>| {
            work();
            true
        };
        let mut job: Job<u32, u32> = Job::default();
        job.ask(&1, || 7, &here, || {});
        assert!(matches!(job.take(), Some((1, Outcome::Made(7)))));
        assert!(job.loading().is_empty());
        job.ask(&1, || panic!("the same rows are not built twice"), &here, || {});
        assert!(job.take().is_none());
        let refused = |_: Box<dyn FnOnce() + Send>| false;
        job.ask(&2, || 8, &refused, || {});
        assert!(!job.running(), "a worker that refuses leaves it to the next frame");
        let held: std::sync::Mutex<Vec<Box<dyn FnOnce() + Send>>> = std::sync::Mutex::default();
        let submit = |work: Box<dyn FnOnce() + Send>| {
            held.lock().unwrap().push(work);
            true
        };
        job.ask(&2, || 8, &submit, || {});
        job.ask(&3, || panic!("one build at a time"), &submit, || {});
        assert!(job.take().is_none(), "nothing until the worker ran it");
        let work = held.lock().unwrap().pop().unwrap();
        std::thread::spawn(work).join().unwrap();
        assert!(matches!(job.take(), Some((2, Outcome::Made(8)))));
    }
}
