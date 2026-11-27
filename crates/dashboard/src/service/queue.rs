use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Condvar, Mutex, atomic::{AtomicUsize, Ordering}, mpsc::TrySendError},
};

pub(super) trait Classed {
    fn class(&self) -> &str;
}

struct State<T> {
    queued: VecDeque<(String, T)>,
    running: HashMap<String, usize>,
    closed: bool,
}
struct Queue<T> {
    state: Mutex<State<T>>,
    changed: Condvar,
    capacity: usize,
    share: usize,
    priority_jobs: Arc<AtomicUsize>,
}
pub(super) struct Sender<T>(Arc<Queue<T>>);
pub(super) struct Receiver<T>(Arc<Queue<T>>);

pub(super) struct Turn<T> {
    queue: Arc<Queue<T>>,
    class: String,
}

pub(super) fn share(threads: usize) -> usize {
    if threads > 2 { threads - 1 } else { threads.max(1) }
}

pub(super) fn channel<T>(capacity: usize, share: usize, priority_jobs: Arc<AtomicUsize>) -> (Sender<T>, Receiver<T>) {
    let queue = Arc::new(Queue {
        state: Mutex::new(State { queued: VecDeque::new(), running: HashMap::new(), closed: false }),
        changed: Condvar::new(),
        capacity,
        share: share.max(1),
        priority_jobs,
    });
    (Sender(Arc::clone(&queue)), Receiver(queue))
}
impl<T: Classed> Sender<T> {
    pub(super) fn try_send(&self, value: T, mut stale: impl FnMut(&T) -> bool) -> Result<(), TrySendError<T>> {
        let mut state = self.0.state.lock().unwrap_or_else(|error| error.into_inner());
        if state.closed {
            return Err(TrySendError::Disconnected(value));
        }
        let mut gone = VecDeque::new();
        if state.queued.len() >= self.0.capacity {
            let kept;
            (kept, gone) = std::mem::take(&mut state.queued).into_iter().partition(|(_, queued)| !stale(queued));
            state.queued = kept;
            self.0.remove_priority(gone.iter().map(|(class, _)| class.as_str()));
        }
        if state.queued.len() >= self.0.capacity {
            return Err(TrySendError::Full(value));
        }
        let class = value.class().to_owned();
        if class != crate::strip::PRODUCT {
            self.0.priority_jobs.fetch_add(1, Ordering::Release);
        }
        state.queued.push_back((class, value));
        self.0.changed.notify_one();
        drop(state);
        drop(gone);
        Ok(())
    }
}
impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        self.0.state.lock().unwrap_or_else(|error| error.into_inner()).closed = true;
        self.0.changed.notify_all();
    }
}
impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        let queued = {
            let mut state = self.0.state.lock().unwrap_or_else(|error| error.into_inner());
            state.closed = true;
            let queued = std::mem::take(&mut state.queued);
            self.0.remove_priority(queued.iter().map(|(class, _)| class.as_str()));
            queued
        };
        self.0.changed.notify_all();
        drop(queued);
    }
}
impl<T> Drop for Turn<T> {
    fn drop(&mut self) {
        let mut state = self.queue.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(running) = state.running.get_mut(&self.class) {
            *running -= 1;
            if *running == 0 {
                state.running.remove(&self.class);
            }
        }
        self.queue.remove_priority(std::iter::once(self.class.as_str()));
        drop(state);
        self.queue.changed.notify_all();
    }
}
impl<T> Queue<T> {
    fn remove_priority<'a>(&self, classes: impl Iterator<Item = &'a str>) {
        let removed = classes.filter(|class| *class != crate::strip::PRODUCT).count();
        if removed != 0 {
            self.priority_jobs.fetch_sub(removed, Ordering::Release);
        }
    }

    fn next(&self, state: &State<T>) -> Option<usize> {
        let mut best: Option<(usize, usize)> = None;
        for (at, (class, _)) in state.queued.iter().enumerate() {
            let running = state.running.get(class).copied().unwrap_or(0);
            if running < self.share && best.is_none_or(|(fewest, _)| running < fewest) {
                best = Some((running, at));
            }
        }
        best.map(|(_, at)| at)
    }
}
impl<T> Receiver<T> {
    #[cfg(test)]
    pub(super) fn recv(&self) -> Result<(T, Turn<T>), ()> {
        self.recv_idle(|| {})
    }

    pub(super) fn recv_idle(&self, idle: impl FnOnce()) -> Result<(T, Turn<T>), ()> {
        let queue = &self.0;
        let lock = || queue.state.lock().unwrap_or_else(|error| error.into_inner());
        let mut idle = Some(idle);
        let mut state = lock();
        loop {
            if let Some((class, value)) = queue.next(&state).and_then(|at| state.queued.remove(at)) {
                *state.running.entry(class.clone()).or_default() += 1;
                return Ok((value, Turn { queue: Arc::clone(queue), class }));
            }
            if state.closed {
                return Err(());
            }
            if let Some(idle) = idle.take() {
                drop(state);
                idle();
                state = lock();
                continue;
            }
            state = queue.changed.wait(state).unwrap_or_else(|error| error.into_inner());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Classed for i32 {
        fn class(&self) -> &str {
            "number"
        }
    }

    impl Classed for (&'static str, i32) {
        fn class(&self) -> &str {
            self.0
        }
    }

    #[test]
    fn values_leave_in_order_and_a_full_queue_refuses() {
        let (sender, receiver) = channel(2, 8, Arc::default());
        let none = |_: &i32| false;
        sender.try_send(1, none).unwrap();
        sender.try_send(2, none).unwrap();
        assert!(matches!(sender.try_send(3, none), Err(TrySendError::Full(3))));
        assert_eq!(receiver.recv().map(|(value, _)| value), Ok(1));
        assert_eq!(receiver.recv().map(|(value, _)| value), Ok(2));
        drop(sender);
        assert!(receiver.recv().is_err());
    }

    #[test]
    fn a_full_queue_lets_go_of_stale_values_for_a_new_one() {
        let priority_jobs = Arc::new(AtomicUsize::new(0));
        let (sender, receiver) = channel(3, 8, Arc::clone(&priority_jobs));
        let odd = |value: &i32| value % 2 == 1;
        for value in [1, 2, 3] {
            sender.try_send(value, odd).unwrap();
        }
        sender.try_send(4, odd).unwrap();
        sender.try_send(5, odd).unwrap();
        assert!(matches!(sender.try_send(6, |_: &i32| false), Err(TrySendError::Full(6))));
        assert_eq!(priority_jobs.load(Ordering::Acquire), 3);
        let taken: Vec<i32> = (0..3).map(|_| receiver.recv().unwrap().0).collect();
        assert_eq!(taken, [2, 4, 5]);
        assert_eq!(priority_jobs.load(Ordering::Acquire), 0);
    }

    #[test]
    fn a_burst_of_one_kind_never_holds_the_lane_from_another() {
        let (sender, receiver) = channel(64, share(4), Arc::default());
        let none = |_: &(&'static str, i32)| false;
        for chunk in 0..14 {
            sender.try_send(("power-spending", chunk), none).unwrap();
        }
        let held: Vec<_> = (0..3).map(|_| receiver.recv().unwrap()).collect();
        assert_eq!(held.iter().map(|(job, _)| job.1).collect::<Vec<_>>(), [0, 1, 2], "one kind alone runs in order on all but one thread");
        sender.try_send(("series-tiles", 0), none).unwrap();
        sender.try_send(("series-tiles", 1), none).unwrap();
        let (quick, quick_turn) = receiver.recv().unwrap();
        assert_eq!(quick, ("series-tiles", 0), "the free thread goes to the kind asked later");
        let (next, _turn) = receiver.recv().unwrap_or_else(|()| panic!("the queue closed"));
        assert_eq!(next, ("series-tiles", 1), "the slow kind is at its share, so the quick one runs again");
        drop(quick_turn);
        drop(held);
        let (resumed, _) = receiver.recv().unwrap();
        assert_eq!(resumed, ("power-spending", 3), "once its turns end, the slow kind goes on in order");
    }

    #[test]
    fn priority_work_counts_queued_and_running_jobs_across_lanes() {
        let priority = Arc::new(AtomicUsize::new(0));
        let (replies, reply_jobs) = channel(4, 2, Arc::clone(&priority));
        let (interactive, interactive_jobs) = channel(4, 2, Arc::clone(&priority));
        replies.try_send((crate::strip::PRODUCT, 0), |_| false).unwrap();
        let (_, strip) = reply_jobs.recv().unwrap();
        assert_eq!(priority.load(Ordering::Acquire), 0);
        interactive.try_send(("foreground", 0), |_| false).unwrap();
        assert_eq!(priority.load(Ordering::Acquire), 1);
        let (_, foreground) = interactive_jobs.recv().unwrap();
        assert_eq!(priority.load(Ordering::Acquire), 1);
        replies.try_send(("foreground", 1), |_| false).unwrap();
        drop(reply_jobs);
        assert_eq!(priority.load(Ordering::Acquire), 1);
        drop((foreground, strip));
        assert_eq!(priority.load(Ordering::Acquire), 0);
    }
}
