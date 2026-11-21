use super::*;
use std::sync::atomic::AtomicPtr;

static HEAD: AtomicPtr<Site> = AtomicPtr::new(std::ptr::null_mut());
pub(super) struct Site {
    calls: AtomicU64,
    intervals: AtomicU64,
    pending: AtomicBool,
    next: AtomicPtr<Site>,
}
impl Site {
    pub(super) const fn new() -> Self {
        Self {
            calls: AtomicU64::new(0),
            intervals: AtomicU64::new(0),
            pending: AtomicBool::new(false),
            next: AtomicPtr::new(std::ptr::null_mut()),
        }
    }
    pub(super) fn hit(&'static self) {
        if self.calls.fetch_add(1, Ordering::Relaxed) == 0 {
            let mut head = HEAD.load(Ordering::Acquire);
            loop {
                self.next.store(head, Ordering::Relaxed);
                match HEAD.compare_exchange_weak(
                    head,
                    std::ptr::from_ref(self).cast_mut(),
                    Ordering::Release,
                    Ordering::Acquire,
                ) {
                    Ok(_) => break,
                    Err(current) => head = current,
                }
            }
        }
        self.pending.store(true, Ordering::Release);
    }
}
fn sites(mut visit: impl FnMut(&Site)) {
    let mut next = HEAD.load(Ordering::Acquire);
    // SAFETY: published nodes have process-static storage and are never removed or freed.
    while let Some(site) = unsafe { next.as_ref() } {
        visit(site);
        next = site.next.load(Ordering::Acquire);
    }
}
pub(super) fn clear() {
    sites(|site| {
        site.pending.store(false, Ordering::Release);
    });
}
pub(super) fn refused() {
    sites(|site| {
        if site.pending.swap(false, Ordering::AcqRel) {
            site.intervals.fetch_add(1, Ordering::Relaxed);
        }
    });
}
static REASONS: OnceLock<Mutex<HashMap<&'static str, u64>>> = OnceLock::new();
pub(super) fn refusal(reason: &'static str) {
    let mut reasons = REASONS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(count) = reasons.get_mut(reason) {
        *count += 1;
    } else if engine_allocation::reserve_map(&mut reasons, 1).is_ok() {
        reasons.insert(reason, 1);
    }
}
macro_rules! missed_site {
    ($cause:literal) => {{
        static SITE: $crate::production::design_changes::diagnostics::Site =
            $crate::production::design_changes::diagnostics::Site::new();
        SITE.hit();
        $crate::production::design_changes::missed_at();
    }};
}
macro_rules! missed {
    () => {
        missed_site!("lost_occurrence")
    };
}
macro_rules! unobserved {
    () => {{
        $crate::production::design_changes::refuse_census();
        missed_site!("unobserved");
    }};
}
macro_rules! unknown_receiver {
    () => {{
        $crate::production::design_changes::UNKNOWN_RECEIVERS
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        $crate::production::design_changes::refuse_census();
        missed_site!("unknown_receiver");
    }};
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missed_sources_count_each_static_path_and_each_refused_interval_without_allocation() {
        static SITE: Site = Site::new();
        collector_test_support::with_allocations_denied(|| {
            SITE.hit();
            SITE.hit();
        });
        refused();
        assert_eq!(SITE.calls.load(Ordering::Relaxed), 2);
        assert_eq!(SITE.intervals.load(Ordering::Relaxed), 1);
        refused();
        assert_eq!(SITE.intervals.load(Ordering::Relaxed), 1);
        SITE.hit();
        refused();
        assert_eq!(SITE.intervals.load(Ordering::Relaxed), 2);
    }
}
