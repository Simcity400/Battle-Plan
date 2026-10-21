//! Per-batch scheduler accounting, enabled only by normal PROFILE.
use super::HelperSource;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Default)]
pub(super) struct Counts {
    protected: AtomicUsize,
    builders: AtomicUsize,
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

    pub fn accepted(&self, source: HelperSource) {
        if let Some(counts) = &self.counts {
            match source {
                HelperSource::Protected => &counts.protected,
                HelperSource::Builder => &counts.builders,
            }
            .fetch_add(1, Ordering::Relaxed);
        }
    }
}

impl Drop for Batch {
    fn drop(&mut self) {
        if let Some(c) = &self.counts {
            crate::profile_writer::profile_print(format!(
                "FRAME_TURNS kind={} requested={} protected_accepts={} builder_fallback_accepts={} admitted_helpers={} started_helpers={} caller_tasks={} beside_hits={}",
                self.kind,
                self.requested,
                c.protected.load(Ordering::Relaxed),
                c.builders.load(Ordering::Relaxed),
                c.admitted.load(Ordering::Relaxed),
                c.started.load(Ordering::Relaxed),
                c.caller.load(Ordering::Relaxed),
                c.beside.load(Ordering::Relaxed),
            ));
        }
    }
}
