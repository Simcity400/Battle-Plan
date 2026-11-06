//! One source-read mailbox per map, used by the decode lane or shared-source actor.
//! Wakes and completion callbacks only post nonblocking events. No source I/O or
//! decode is executed while holding this mailbox lock or by a callback.
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
};

pub(super) enum Reply<V, S> {
    Ready { value: V, sample: Option<S> },
    Cancelled,
    Stopped,
}

type Complete<V, S> = Box<dyn FnOnce(u64, Reply<V, S>) + Send>;

pub(super) struct Read<K, V, S> {
    pub id: u64,
    pub key: K,
    cancelled: Arc<AtomicBool>,
    complete: Option<Complete<V, S>>,
}

impl<K, V, S> Read<K, V, S> {
    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn ready(mut self, value: V, sample: Option<S>) {
        self.emit(Reply::Ready { value, sample });
    }

    fn emit(&mut self, reply: Reply<V, S>) {
        if let Some(complete) = self.complete.take() {
            let reply = if self.cancelled() {
                Reply::Cancelled
            } else {
                reply
            };
            // A notification failure cannot kill the serial source service.
            // The concrete callback only sends an Event and never unwraps it.
            let _ =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| complete(self.id, reply)));
        }
    }
}

impl<K, V, S> Drop for Read<K, V, S> {
    fn drop(&mut self) {
        self.emit(Reply::Stopped);
    }
}

struct State<K, V, S> {
    pending: Option<Read<K, V, S>>,
    /// A queued wake remains valid even when its request is cancelled/replaced.
    /// Keep it until take(), rather than enqueueing one wake for every Want.
    queued: bool,
    stopped: bool,
}

pub(super) struct Mailbox<K, V, S> {
    state: Mutex<State<K, V, S>>,
    wake: Box<dyn Fn(Arc<Self>) -> bool + Send + Sync>,
}

pub(super) struct ReadHandle<K, V, S> {
    id: u64,
    cancelled: Arc<AtomicBool>,
    mailbox: Weak<Mailbox<K, V, S>>,
}

impl<K, V, S> ReadHandle<K, V, S> {
    /// Only source requests are cancelled here, never a pool-accepted job.
    /// A queued read acknowledges immediately via its terminal callback. An
    /// executing read acknowledges at its ordinary service completion boundary.
    /// In either case the coordinator retains its credit until that reply.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        let pending = self.mailbox.upgrade().and_then(|mailbox| {
            let mut state = mailbox.state.lock().unwrap_or_else(|p| p.into_inner());
            if state.pending.as_ref().is_some_and(|read| {
                read.id == self.id && Arc::ptr_eq(&read.cancelled, &self.cancelled)
            }) {
                state.pending.take()
            } else {
                None
            }
        });
        // Callback after releasing the mailbox lock, including reentrant cases.
        drop(pending);
    }
}

impl<K, V, S> Mailbox<K, V, S> {
    pub fn new(wake: impl Fn(Arc<Self>) -> bool + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State {
                pending: None,
                queued: false,
                stopped: false,
            }),
            wake: Box::new(wake),
        })
    }

    pub fn post(
        self: &Arc<Self>,
        id: u64,
        key: K,
        complete: impl FnOnce(u64, Reply<V, S>) + Send + 'static,
    ) -> ReadHandle<K, V, S> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let (old, wake, replaced) = {
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            let read = Read {
                id,
                key,
                cancelled: cancelled.clone(),
                complete: Some(Box::new(complete)),
            };
            if state.stopped {
                (Some(read), false, false)
            } else {
                let old = state.pending.replace(read);
                let wake = !state.queued;
                state.queued = true;
                (old, wake, true)
            }
        };
        if replaced && let Some(old) = &old {
            old.cancelled.store(true, Ordering::Release);
        }
        drop(old);
        let handle = ReadHandle {
            id,
            cancelled,
            mailbox: Arc::downgrade(self),
        };
        if wake && !(self.wake)(self.clone()) {
            self.stop();
        }
        handle
    }

    /// Service-thread only. For a Condvar-backed lane, call under its existing
    /// LaneState lock; the wake callback acquires that lock before notify, so a
    /// request cannot land between the empty check and the atomic sleep.
    pub fn take(&self) -> Option<Read<K, V, S>> {
        let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        state.queued = false;
        state.pending.take()
    }

    /// Short metadata read; never decode or clone a table body here.
    pub fn pending_key(&self) -> Option<K>
    where
        K: Clone,
    {
        self.state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .pending
            .as_ref()
            .map(|read| read.key.clone())
    }

    /// The service exit guard calls this, including on panic. Actor registries
    /// snapshot weak clients before calling it, outside their own registry lock.
    pub fn stop(&self) {
        let pending = {
            let mut state = self.state.lock().unwrap_or_else(|p| p.into_inner());
            state.queued = false;
            state.stopped = true;
            state.pending.take()
        };
        drop(pending);
    }
}

#[cfg(test)]
mod tests;
