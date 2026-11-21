use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

#[derive(Default)]
pub(super) struct Capacity {
    generation: AtomicU64,
    listener_count: AtomicUsize,
    listeners: Mutex<Vec<Weak<Signal>>>,
}
struct Signal {
    pending: AtomicBool,
    wake: Box<dyn Fn() + Send + Sync>,
}

pub struct CapacityListener {
    capacity: Arc<Capacity>,
    signal: Arc<Signal>,
}
impl CapacityListener {
    pub fn acknowledge(&self) -> u64 {
        self.signal.pending.store(false, Ordering::Release);
        self.generation()
    }
    pub fn generation(&self) -> u64 {
        self.capacity.generation.load(Ordering::Acquire)
    }
}
impl Drop for CapacityListener {
    fn drop(&mut self) {
        let mut listeners = self
            .capacity
            .listeners
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        let own = Arc::downgrade(&self.signal);
        listeners.retain(|entry| !entry.ptr_eq(&own));
        self.capacity
            .listener_count
            .store(listeners.len(), Ordering::Release);
    }
}
impl Capacity {
    pub(super) fn listen(
        self: &Arc<Self>,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> CapacityListener {
        let signal = Arc::new(Signal {
            pending: AtomicBool::new(false),
            wake: Box::new(wake),
        });
        let mut listeners = self.listeners.lock().unwrap_or_else(|p| p.into_inner());
        listeners.push(Arc::downgrade(&signal));
        self.listener_count
            .store(listeners.len(), Ordering::Release);
        CapacityListener {
            capacity: self.clone(),
            signal,
        }
    }

    pub(super) fn dequeued(&self) {
        if self.listener_count.load(Ordering::Acquire) == 0 {
            return;
        }
        self.generation.fetch_add(1, Ordering::AcqRel);
        let listeners: Vec<_> = self
            .listeners
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .iter()
            .filter_map(Weak::upgrade)
            .collect();
        for listener in listeners {
            if !listener.pending.swap(true, Ordering::AcqRel)
                && std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (listener.wake)()))
                    .is_err()
            {
                listener.pending.store(false, Ordering::Release);
            }
        }
    }
}
