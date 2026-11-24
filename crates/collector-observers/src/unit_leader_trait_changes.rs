use crate::{
    unit_leader_changes::{Changes, Counts},
    watch_registry::{Notification, WatchRegistry},
};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

static WATCHES: WatchRegistry = WatchRegistry::new();
static CALLS: [AtomicU64; Site::ALL.len()] = [const { AtomicU64::new(0) }; Site::ALL.len()];
static READS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static LEADERS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Site {
    RecalculateModifiers,
    Construct,
    CopyConstruct,
    Destroy,
}

impl Site {
    const ALL: [Self; 4] = [
        Self::RecalculateModifiers,
        Self::Construct,
        Self::CopyConstruct,
        Self::Destroy,
    ];

}

pub struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        WATCHES.quiet()
    }
    fn sequence(&self, leader: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(leader)
    }
}

pub fn active() -> usize {
    WATCHES.active()
}

pub fn boundary(invalidate: bool) {
    WATCHES.boundary(invalidate);
}

pub fn presize() {
    let leaders = LEADERS.load(Ordering::Relaxed);
    if leaders != 0 {
        WATCHES.offer(leaders);
    }
}

pub fn collect() {
    WATCHES.collect();
}

pub fn forwarder(enabled: bool) {
    WATCHES.set_enabled(cfg!(feature = "unit_leader_trait_changes") && enabled);
}

/// Free the watch tables once the forwarding patches were removed and
/// drained after the last capture.
///
/// # Safety
/// No notification may still hold a table.
pub unsafe fn release() {
    unsafe { WATCHES.release() };
}

pub fn forwarded(leader: usize, site: Site) -> Notification<'static> {
    CALLS[site as usize].fetch_add(1, Ordering::Relaxed);
    WATCHES.notify(leader)
}

pub fn record(counts: &Counts, leaders: usize) {
    for (target, value) in
        READS
            .iter()
            .zip([counts.reused, counts.read, u64::from(counts.reused != 0)])
    {
        target.fetch_add(value, Ordering::Relaxed);
    }
    LEADERS.store(leaders, Ordering::Relaxed);
}

