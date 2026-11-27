
use crate::landed::{Landed, Published};
use job_priority::SpawnCarried;
use replay_data::rows::TableId;
use std::{
    sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError},
    thread::JoinHandle,
    time::{Duration, Instant},
};

type Job<T> = Box<dyn FnOnce() -> T + Send>;
type Trash = Box<dyn Send>;

struct State<K, T> {
    queued: Option<(K, Job<T>)>,
    running: Option<K>,
    done: Published<Option<(K, T)>>,
    missed: Published<Vec<TableId>>,
    timing: Option<Vec<u64>>,
    trash: Vec<Trash>,
    stop: bool,
}

type Shared<K, T> = Arc<(Mutex<State<K, T>>, Condvar)>;

fn lock<K, T>(shared: &Shared<K, T>) -> MutexGuard<'_, State<K, T>> {
    shared.0.lock().unwrap_or_else(PoisonError::into_inner)
}

pub struct Reads<K, T> {
    shared: Shared<K, T>,
    worker: Option<JoinHandle<()>>,
    name: &'static str,
}

impl<K: PartialEq + Clone + Send + 'static, T: Send + 'static> Reads<K, T> {
    pub fn new(name: &'static str) -> Self {
        let state = State { queued: None, running: None, done: Published::default(), missed: Published::default(), timing: None, trash: Vec::new(), stop: false };
        Self { shared: Arc::new((Mutex::new(state), Condvar::new())), worker: None, name }
    }

    pub fn ask(&mut self, key: K, job: impl FnOnce() -> T + Send + 'static, wake: impl Fn() + Send + 'static) {
        if self.worker.as_ref().is_some_and(JoinHandle::is_finished)
            && let Some(Err(panic)) = self.worker.take().map(JoinHandle::join)
        {
            std::panic::resume_unwind(panic);
        }
        let mut state = lock(&self.shared);
        let known = |held: Option<&K>| held.is_some_and(|held| *held == key);
        if known(state.done.as_ref().map(|(held, _)| held)) || known(state.running.as_ref()) || known(state.queued.as_ref().map(|(held, _)| held)) {
            if self.worker.is_some() {
                state.trash.push(Box::new(job));
            }
            return;
        }
        if let Some((_, replaced)) = state.queued.replace((key, Box::new(job))) {
            state.trash.push(Box::new(replaced));
        }
        drop(state);
        self.shared.1.notify_all();
        if self.worker.is_none() {
            let shared = Arc::clone(&self.shared);
            self.worker = std::thread::Builder::new().name(self.name.into()).spawn_carried(move || run(&shared, wake)).ok();
        }
    }

    pub fn latest(&self, keep: impl FnOnce(&K) -> bool) -> Option<(K, Landed<T>)>
    where
        T: Clone,
    {
        let state = lock(&self.shared);
        let (key, _) = state.done.as_ref().filter(|(key, _)| keep(key))?;
        let key = key.clone();
        let landed = state.done.landed_clone()?.map(|(_, value)| value);
        Some((key, landed))
    }

    pub fn take(&mut self, keep: impl FnOnce(&K) -> bool) -> Option<(K, Landed<T>)> {
        let mut state = lock(&self.shared);
        if state.done.as_ref().is_some_and(|(key, _)| keep(key)) {
            let (key, value) = state.done.take()?.split();
            Some((key.into_inner(), value))
        } else {
            None
        }
    }

    pub fn wait(&self, keep: impl Fn(&K) -> bool, limit: Duration) -> bool {
        let until = Instant::now() + limit;
        let mut state = lock(&self.shared);
        loop {
            if state.done.as_ref().is_some_and(|(key, _)| keep(key)) {
                return true;
            }
            let now = Instant::now();
            if now >= until || (state.running.is_none() && state.queued.is_none()) {
                return false;
            }
            state = self.shared.1.wait_timeout(state, until - now).unwrap_or_else(PoisonError::into_inner).0;
        }
    }

    pub fn busy(&self) -> bool {
        let state = lock(&self.shared);
        state.running.is_some() || state.queued.is_some()
    }

    pub fn discard(&self, value: impl Send + 'static) {
        if self.worker.is_none() {
            return;
        }
        lock(&self.shared).trash.push(Box::new(value));
        self.shared.1.notify_all();
    }

    pub fn time(&self) {
        lock(&self.shared).timing.get_or_insert_with(Vec::new);
    }

    pub fn times(&self) -> Vec<u64> {
        lock(&self.shared).timing.clone().unwrap_or_default()
    }

    pub fn take_missed(&mut self) -> Vec<TableId> {
        lock(&self.shared).missed.take_all().map(Landed::into_inner).unwrap_or_default()
    }
}

impl<K, T> Drop for Reads<K, T> {
    fn drop(&mut self) {
        lock(&self.shared).stop = true;
        self.shared.1.notify_all();
    }
}

fn run<K: Clone, T>(shared: &Shared<K, T>, wake: impl Fn()) {
    loop {
        let (key, job) = {
            let mut state = lock(shared);
            loop {
                if state.stop {
                    return;
                }
                if !state.trash.is_empty() {
                    let trash = std::mem::take(&mut state.trash);
                    drop(state);
                    drop(trash);
                    state = lock(shared);
                    continue;
                }
                if let Some(next) = state.queued.take() {
                    state.running = Some(next.0.clone());
                    break next;
                }
                state = shared.1.wait(state).unwrap_or_else(PoisonError::into_inner);
            }
        };
        let started = std::time::Instant::now();
        let (value, missed) = replay_frame::frame::discovering(job);
        let spent = started.elapsed().as_nanos() as u64;
        let mut state = lock(shared);
        state.running = None;
        if let Some(timing) = &mut state.timing {
            timing.push(spent);
        }
        if missed.is_empty() {
            state.done.set_some((key, value));
        } else {
            let novel: Vec<TableId> = missed.into_iter().filter(|table| !state.missed.get().contains(table)).collect();
            state.missed.extend_nonempty(novel);
        }
        drop(state);
        shared.1.notify_all();
        wake();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, time::Duration};

    #[test]
    fn a_result_is_kept_by_its_key_asked_once_and_a_read_over_an_undeclared_table_keeps_nothing() {
        let mut reads: Reads<u32, u32> = Reads::new("reads test");
        let (done, wait) = mpsc::channel();
        let next = || wait.recv_timeout(Duration::from_secs(60)).expect("the worker answers");
        let before = crate::landed::count();
        reads.ask(1, || 10, move || {
            let _ = done.send(());
        });
        next();
        assert!(crate::landed::count() > before, "publishing a result moves the landed count");
        assert_eq!(reads.latest(|key| *key == 1).map(|(k, v)| (k, v.into_inner())), Some((1, 10)));
        reads.ask(1, || panic!("a key made already is not made again"), || {});
        assert!(!reads.busy(), "nothing is queued for it");
        assert_eq!(reads.latest(|key| *key == 2), None, "another key's result is not this one's");
        reads.ask(2, || 20, || {});
        next();
        assert_eq!(reads.take(|key| *key == 2).map(|(k, v)| (k, v.into_inner())), Some((2, 20)));
        assert_eq!(reads.take(|_| true), None, "taken once");
        let before_missed = crate::landed::count();
        let undeclared = replay_frame::frame::not_declared(TableId::Countries);
        reads.ask(3, move || {
            let _ = &*undeclared;
            30
        }, || {});
        next();
        assert!(crate::landed::count() > before_missed, "publishing missed tables moves the landed count");
        assert_eq!(reads.latest(|_| true), None, "a read over an undeclared table is void");
        assert_eq!(reads.take_missed(), [TableId::Countries]);
        assert!(reads.take_missed().is_empty());
    }

    #[test]
    fn a_job_that_panicked_is_raised_on_the_next_ask_not_left_answering_nothing() {
        let mut reads: Reads<u32, u32> = Reads::new("reads panic test");
        reads.ask(1, || panic!("the leaf failed"), || {});
        while !reads.worker.as_ref().is_some_and(JoinHandle::is_finished) {
            std::thread::yield_now();
        }
        let raised = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| reads.ask(2, || 20, || {})));
        let message = raised.expect_err("the worker's panic reaches the asking thread");
        assert_eq!(message.downcast_ref::<&str>(), Some(&"the leaf failed"));
    }
}
