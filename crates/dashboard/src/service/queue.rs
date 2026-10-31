//! Bounded loader queues. FIFO within each class; first asks have bounded priority.
use std::{
    collections::VecDeque,
    sync::{Arc, Condvar, Mutex, mpsc::TrySendError},
};

/// A waiting refresh runs after at most this many consecutive first asks.
/// The browser connection queue uses the same admission rule.
pub(super) const FIRST_ASK_BURST: usize = 8;

struct State<T> {
    first: VecDeque<T>,
    refresh: VecDeque<T>,
    closed: bool,
    first_burst: usize,
}
struct Queue<T> {
    state: Mutex<State<T>>,
    changed: Condvar,
    capacity: usize,
}
/// One non-cloneable sender, owned by `Lanes`; producers borrow it.
pub(super) struct Sender<T>(Arc<Queue<T>>);
pub(super) struct Receiver<T>(Arc<Queue<T>>);

pub(super) fn channel<T>(capacity: usize) -> (Sender<T>, Receiver<T>) {
    let queue = Arc::new(Queue {
        state: Mutex::new(State {
            first: VecDeque::new(),
            refresh: VecDeque::new(),
            closed: false,
            first_burst: 0,
        }),
        changed: Condvar::new(),
        capacity,
    });
    (Sender(Arc::clone(&queue)), Receiver(queue))
}
impl<T> Sender<T> {
    pub(super) fn try_send(&self, refresh: bool, value: T) -> Result<(), TrySendError<T>> {
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.closed {
            return Err(TrySendError::Disconnected(value));
        }
        if state.first.len() + state.refresh.len() >= self.0.capacity {
            return Err(TrySendError::Full(value));
        }
        if refresh {
            state.refresh.push_back(value);
        } else {
            state.first.push_back(value);
        }
        self.0.changed.notify_one();
        Ok(())
    }
}
impl<T> Drop for Sender<T> {
    fn drop(&mut self) {
        self.0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .closed = true;
        self.0.changed.notify_all();
    }
}
impl<T> Drop for Receiver<T> {
    fn drop(&mut self) {
        let queued = {
            let mut state = self
                .0
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state.closed = true;
            (
                std::mem::take(&mut state.first),
                std::mem::take(&mut state.refresh),
            )
        };
        self.0.changed.notify_all();
        drop(queued);
    }
}
impl<T> Receiver<T> {
    #[cfg(test)]
    pub(super) fn recv(&self) -> Result<T, ()> {
        self.recv_idle(|| {})
    }

    /// The next value, calling `idle` once, without the lock, before it first waits for one.
    pub(super) fn recv_idle(&self, idle: impl FnOnce()) -> Result<T, ()> {
        let lock = || {
            self.0
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
        };
        let mut idle = Some(idle);
        let mut state = lock();
        loop {
            let refresh_due = !state.refresh.is_empty()
                && (state.first.is_empty() || state.first_burst >= FIRST_ASK_BURST);
            let value = if refresh_due {
                state.first_burst = 0;
                state.refresh.pop_front()
            } else {
                if state.refresh.is_empty() {
                    state.first_burst = 0;
                } else if !state.first.is_empty() {
                    state.first_burst += 1;
                }
                state.first.pop_front()
            };
            if let Some(value) = value {
                return Ok(value);
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
            state = self
                .0
                .changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn first_asks_overtake_refreshes_and_each_class_stays_fifo() {
        let (sender, receiver) = channel(4);
        sender.try_send(true, 1).unwrap();
        sender.try_send(false, 2).unwrap();
        sender.try_send(true, 3).unwrap();
        sender.try_send(false, 4).unwrap();
        assert!(matches!(
            sender.try_send(false, 5),
            Err(TrySendError::Full(5))
        ));
        assert_eq!(
            (
                receiver.recv(),
                receiver.recv(),
                receiver.recv(),
                receiver.recv()
            ),
            (Ok(2), Ok(4), Ok(1), Ok(3))
        );
        drop(sender);
        assert_eq!(receiver.recv(), Err(()));
    }

    #[test]
    fn sustained_first_asks_cannot_starve_waiting_refreshes() {
        assert_eq!(
            FIRST_ASK_BURST, 8,
            "keep the browser and service admission bound equal"
        );
        let (sender, receiver) = channel(FIRST_ASK_BURST + 2);
        sender.try_send(true, ("refresh", 0)).unwrap();
        sender.try_send(true, ("refresh", 1)).unwrap();
        for index in 0..FIRST_ASK_BURST {
            sender.try_send(false, ("first", index)).unwrap();
        }
        let mut next = FIRST_ASK_BURST;
        for round in 0..2 {
            for index in 0..FIRST_ASK_BURST {
                assert_eq!(
                    receiver.recv(),
                    Ok(("first", round * FIRST_ASK_BURST + index))
                );
                sender.try_send(false, ("first", next)).unwrap();
                next += 1;
            }
            assert_eq!(
                receiver.recv(),
                Ok(("refresh", round)),
                "first asks starved a refresh"
            );
            sender.try_send(false, ("first", next)).unwrap();
            next += 1;
        }
    }

    #[test]
    fn dropping_the_receiver_disconnects_sends_even_when_the_queue_was_full() {
        for queued in [0, 2] {
            let (sender, receiver) = channel(2);
            let value = Arc::new(());
            for refresh in [false, true].into_iter().take(queued) {
                sender.try_send(refresh, Arc::clone(&value)).unwrap();
            }
            drop(receiver);
            assert_eq!(
                Arc::strong_count(&value),
                1,
                "queued work outlived its receiver"
            );
            for refresh in [false, true] {
                assert!(matches!(
                    sender.try_send(refresh, Arc::clone(&value)),
                    Err(TrySendError::Disconnected(_))
                ));
            }
        }
    }

    #[test]
    fn workers_share_one_receiver_and_only_its_last_owner_disconnects() {
        let (sender, receiver) = channel(1);
        let workers = Arc::new(Mutex::new(receiver));
        let other_worker = Arc::clone(&workers);
        drop(workers);
        sender.try_send(false, 1).unwrap();
        assert_eq!(other_worker.lock().unwrap().recv(), Ok(1));
        drop(other_worker);
        assert!(matches!(
            sender.try_send(false, 2),
            Err(TrySendError::Disconnected(2))
        ));
    }
}
