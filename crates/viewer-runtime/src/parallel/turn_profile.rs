use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
pub(super) struct Counts {
    accepted: AtomicUsize,
    pub admitted: AtomicUsize,
    pub started: AtomicUsize,
    pub caller: AtomicUsize,
    pub beside: AtomicUsize,
}

pub(super) struct Batch {
    kind: &'static str,
    requested: usize,
    pub counts: Option<Arc<Counts>>,
}

impl Batch {
    pub fn new(kind: &'static str, requested: usize) -> Self {
        Self {
            kind,
            requested,
            counts: crate::profile_writer::profiling().then(|| Arc::new(Counts::default())),
        }
    }

    pub fn accepted(&self) {
        if let Some(counts) = &self.counts {
            counts.accepted.fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl Drop for Batch {
    fn drop(&mut self) {
        if let Some(c) = &self.counts {
            crate::profile_writer::profile_print(format!(
                "FRAME_TURNS kind={} requested={} accepts={} admitted_helpers={} started_helpers={} caller_tasks={} beside_hits={}",
                self.kind,
                self.requested,
                c.accepted.load(Ordering::Relaxed),
                c.admitted.load(Ordering::Relaxed),
                c.started.load(Ordering::Relaxed),
                c.caller.load(Ordering::Relaxed),
                c.beside.load(Ordering::Relaxed),
            ));
        }
    }
}
