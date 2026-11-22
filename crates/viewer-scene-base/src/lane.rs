
use job_priority::SpawnCarried as _;
use std::{
    ops::{Deref, DerefMut},
    panic::{AssertUnwindSafe, catch_unwind, resume_unwind},
    sync::mpsc,
    thread,
};

type Job<T> = Box<dyn FnOnce() -> T + Send>;

struct Worker<T> {
    jobs: mpsc::Sender<Job<T>>,
    done: mpsc::Receiver<thread::Result<T>>,
}

pub struct Lane<T> {
    name: &'static str,
    worker: Option<Worker<T>>,
    refused: bool,
    started: Started<T>,
}

enum Started<T> {
    None,
    Away,
    Here(thread::Result<T>),
}

impl<T: Send + 'static> Lane<T> {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            worker: None,
            refused: false,
            started: Started::None,
        }
    }

    pub fn start(&mut self, job: impl FnOnce() -> T + Send + 'static) {
        debug_assert!(
            matches!(self.started, Started::None),
            "the {} job started before was not taken",
            self.name
        );
        drop(self.take());
        let mut job: Job<T> = Box::new(job);
        if let Some(worker) = self.worker() {
            match worker.jobs.send(job) {
                Ok(()) => {
                    self.started = Started::Away;
                    return;
                }
                Err(refused) => {
                    job = refused.0;
                    self.worker = None;
                    self.refused = true;
                }
            }
        }
        self.started = Started::Here(catch_unwind(AssertUnwindSafe(job)));
    }

    pub fn take(&mut self) -> Option<T> {
        let result = match std::mem::replace(&mut self.started, Started::None) {
            Started::None => return None,
            Started::Here(result) => result,
            Started::Away => match self.worker.as_ref().map(|worker| worker.done.recv()) {
                Some(Ok(result)) => result,
                _ => {
                    self.worker = None;
                    self.refused = true;
                    panic!("the {} thread ended with a job under way", self.name)
                }
            },
        };
        match result {
            Ok(result) => Some(result),
            Err(_) if thread::panicking() => None,
            Err(panic) => resume_unwind(panic),
        }
    }

    fn worker(&mut self) -> Option<&Worker<T>> {
        if self.worker.is_none() && !self.refused {
            let (jobs, inbox) = mpsc::channel::<Job<T>>();
            let (finished, done) = mpsc::channel();
            let name = self.name;
            let spawned = thread::Builder::new()
                .name(self.name.to_owned())
                .spawn_carried(move || {
                    let _cost = replay_frame::cpu::scope(name, Some("lane"));
                    viewer_runtime::worker_priority::frame_priority();
                    while let Ok(job) = inbox.recv() {
                        if finished.send(catch_unwind(AssertUnwindSafe(job))).is_err() {
                            return;
                        }
                    }
                });
            match spawned {
                Ok(_) => self.worker = Some(Worker { jobs, done }),
                Err(_) => self.refused = true,
            }
        }
        self.worker.as_ref()
    }
}

pub struct Frame<'a, O> {
    owner: &'a mut O,
    join: fn(&mut O),
}

impl<'a, O> Frame<'a, O> {
    pub fn new(owner: &'a mut O, join: fn(&mut O)) -> Self {
        Self { owner, join }
    }
}

impl<O> Deref for Frame<'_, O> {
    type Target = O;
    fn deref(&self) -> &O {
        self.owner
    }
}

impl<O> DerefMut for Frame<'_, O> {
    fn deref_mut(&mut self) -> &mut O {
        self.owner
    }
}

impl<O> Drop for Frame<'_, O> {
    fn drop(&mut self) {
        (self.join)(self.owner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_runs_on_the_lane_s_thread_and_its_result_is_taken_once() {
        let mut lane = Lane::new("test lane");
        assert!(lane.take().is_none());
        let caller = thread::current().id();
        for round in 0..3u32 {
            let state: Vec<u32> = (0..round).collect();
            lane.start(move || {
                let mut state = state;
                state.push(round * 10);
                (state, thread::current().id())
            });
            let (state, ran_on) = lane.take().expect("a started job gives its result");
            assert_ne!(ran_on, caller);
            assert_eq!(state.last(), Some(&(round * 10)));
            assert_eq!(state.len(), round as usize + 1);
            assert!(lane.take().is_none(), "a result is taken once");
        }
    }

    #[test]
    fn a_refused_thread_runs_jobs_in_place() {
        let mut refused = Lane::new("test lane");
        refused.refused = true;
        let caller = thread::current().id();
        refused.start(move || thread::current().id());
        assert_eq!(refused.take(), Some(caller));
    }

    #[test]
    fn a_job_s_panic_reaches_the_taker_and_the_lane_runs_later_jobs() {
        let mut lane: Lane<u32> = Lane::new("test lane");
        lane.start(|| panic!("job failed"));
        let caught = catch_unwind(AssertUnwindSafe(|| lane.take()));
        assert!(caught.is_err());
        lane.start(|| 7);
        assert_eq!(lane.take(), Some(7));
    }

    struct Owner<T> {
        lane: Lane<T>,
    }

    fn join<T: Send + 'static>(owner: &mut Owner<T>) {
        drop(owner.lane.take());
    }

    fn refused_frame<T: Send + 'static>(
        owner: &mut Owner<T>,
        job: impl FnOnce() -> T + Send + 'static,
    ) -> Result<(), &'static str> {
        let mut frame = Frame::new(owner, join);
        frame.lane.start(job);
        Err("refused before the frame took its job")
    }

    #[test]
    fn a_job_s_panic_is_its_frame_s_though_the_frame_returns_early() {
        let mut owner: Owner<u32> = Owner {
            lane: Lane::new("test lane"),
        };
        let returned = catch_unwind(AssertUnwindSafe(|| {
            refused_frame(&mut owner, || panic!("job failed"))
        }));
        assert!(returned.is_err(), "the job's panic surfaces in the frame that started it");
        let mut frame = Frame::new(&mut owner, join);
        frame.lane.start(|| 7);
        assert_eq!(frame.lane.take(), Some(7));
    }

    #[test]
    fn a_frame_that_returns_early_leaves_nothing_held() {
        let held = std::sync::Arc::new(());
        let mut owner = Owner {
            lane: Lane::new("test lane"),
        };
        let job = std::sync::Arc::clone(&held);
        assert!(refused_frame(&mut owner, move || job).is_err());
        assert_eq!(std::sync::Arc::strong_count(&held), 1);
        assert!(matches!(owner.lane.started, Started::None));
    }
}
