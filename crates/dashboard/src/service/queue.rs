//! Bounded loader queues that share their lane's threads between kinds of work: the next job is
//! the oldest of the kind running on fewest threads, and no kind holds more than its share, so a
//! burst of one product's slow replies never makes another product's quick ones wait behind it.
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Condvar, Mutex, mpsc::TrySendError},
};

/// The kind of work a job is, whose jobs share the lane's threads as one.
pub(super) trait Classed {
    fn class(&self) -> &str;
}

struct State<T> {
    queued: VecDeque<(String, T)>,
    /// Jobs of each kind taken and not yet finished ([`Turn`]).
    running: HashMap<String, usize>,
    closed: bool,
}
struct Queue<T> {
    state: Mutex<State<T>>,
    changed: Condvar,
    capacity: usize,
    /// The most jobs of one kind running at once.
    share: usize,
}
/// One non-cloneable sender, owned by `Lanes`; producers borrow it.
pub(super) struct Sender<T>(Arc<Queue<T>>);
pub(super) struct Receiver<T>(Arc<Queue<T>>);

/// A job's hold on its kind's share of the lane, let go when it is dropped, as the job ends.
pub(super) struct Turn<T> {
    queue: Arc<Queue<T>>,
    class: String,
}

/// The most jobs of one kind a lane of `threads` runs at once: all but one of its threads, so
/// another kind's job always finds one free, except in a lane too small to spare one.
pub(super) fn share(threads: usize) -> usize {
    if threads > 2 { threads - 1 } else { threads.max(1) }
}

pub(super) fn channel<T>(capacity: usize, share: usize) -> (Sender<T>, Receiver<T>) {
    let queue = Arc::new(Queue {
        state: Mutex::new(State { queued: VecDeque::new(), running: HashMap::new(), closed: false }),
        changed: Condvar::new(),
        capacity,
        share: share.max(1),
    });
    (Sender(Arc::clone(&queue)), Receiver(queue))
}
impl<T: Classed> Sender<T> {
    /// Queues `value`; a full queue first lets go of the values `stale` names (work no one waits
    /// for any more), so a lane whose threads are held long never refuses for want of room
    /// that cancelled work holds.
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
        }
        if state.queued.len() >= self.0.capacity {
            return Err(TrySendError::Full(value));
        }
        state.queued.push_back((value.class().to_owned(), value));
        self.0.changed.notify_one();
        // What was let go drops after the lock, as `Receiver`'s drop does.
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
            std::mem::take(&mut state.queued)
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
        drop(state);
        self.queue.changed.notify_all();
    }
}
impl<T> Queue<T> {
    /// The queued job to run next: of the kinds under their share, the oldest job of the one
    /// running on fewest threads.
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

    /// The next job and its turn, calling `idle` once, without the lock, before it first waits.
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
        let (sender, receiver) = channel(2, 8);
        let none = |_: &i32| false;
        sender.try_send(1, none).unwrap();
        sender.try_send(2, none).unwrap();
        assert!(matches!(sender.try_send(3, none), Err(TrySendError::Full(3))));
        assert_eq!(receiver.recv().map(|(value, _)| value), Ok(1));
        assert_eq!(receiver.recv().map(|(value, _)| value), Ok(2));
        drop(sender);
        assert!(receiver.recv().is_err());
    }

    /// A full queue lets go of stale values to take a new one, the rest kept in order; with
    /// room, nothing is let go.
    #[test]
    fn a_full_queue_lets_go_of_stale_values_for_a_new_one() {
        let (sender, receiver) = channel(3, 8);
        let odd = |value: &i32| value % 2 == 1;
        for value in [1, 2, 3] {
            sender.try_send(value, odd).unwrap();
        }
        sender.try_send(4, odd).unwrap();
        sender.try_send(5, odd).unwrap();
        assert!(matches!(sender.try_send(6, |_: &i32| false), Err(TrySendError::Full(6))));
        let taken: Vec<i32> = (0..3).map(|_| receiver.recv().unwrap().0).collect();
        assert_eq!(taken, [2, 4, 5]);
    }

    /// Run 14: fourteen slow chunks of one product held every reply thread, and quick replies
    /// asked after them waited seconds. A kind asked later goes before a kind already running,
    /// and no kind takes the lane's last free thread.
    #[test]
    fn a_burst_of_one_kind_never_holds_the_lane_from_another() {
        let (sender, receiver) = channel(64, share(4));
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
}
