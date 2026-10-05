//! Allocation-free notifications from the native writers of the country air
//! histories AirNavalPatrolHistories records ([`engine::air_history_mutations`]).
//! A detour only notifies the shared watch registry ([`WatchRegistry`]),
//! counts its writer site and calls the original; it never changes game
//! behaviour. Receivers are CStrategicAir objects and histories the collector
//! registered; a notification never infers an owner. Anything unobserved
//! (disabled, poisoned, a refused registration, an epoch change) makes the
//! collector read afresh.
use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine::{Memory, air_history_mutations::MUTATIONS};
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; 2] = [const { AtomicUsize::new(0) }; 2];
static READS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
/// Hours that copied every held history afresh, by cause: an invalidation,
/// a larger watch table, or the hour before failing or being dropped.
static RESTARTS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];

/// One observation domain: the watched receivers and each writer site's
/// calls. Its table is cleared only on an invalidation: a clear starts an
/// epoch, after which every held history would be copied again in one hour.
pub(crate) struct Watches {
    registry: WatchRegistry,
    calls: [AtomicU64; 2],
}

impl Watches {
    pub(crate) const fn new() -> Self {
        Self {
            registry: WatchRegistry::retaining(),
            calls: [const { AtomicU64::new(0) }; 2],
        }
    }

    /// Hook path, allocation-free: a writer of `site` ([`MUTATIONS`] index)
    /// is about to change `receiver`. Dropping the guard notifies its return.
    pub(crate) fn notify(&self, receiver: usize, site: usize) -> Notification<'_> {
        let notification = self.registry.notify(receiver);
        // Counted while the notification runs, so a site certificate taken
        // with no writer running holds every call begun before it.
        self.calls[site].fetch_add(1, Ordering::SeqCst);
        notification
    }

    /// Off the game thread: room for `receivers`, and for every receiver
    /// registered since the last clear, as a table a quarter full.
    fn offer(&self, receivers: usize) {
        self.registry
            .offer(receivers.max(self.registry.registered()));
    }

    /// An enabled domain with a table for `slots` country slots and
    /// `regions` strategic regions installed.
    #[cfg(test)]
    pub(crate) fn enabled(slots: usize, regions: usize) -> Self {
        let watches = Self::new();
        watches.offer(watches_for(slots, regions));
        watches.registry.boundary(false);
        watches.registry.set_enabled(true);
        watches
    }

    /// Grow as the spool does from an hour's `receivers`, installing a
    /// larger table at the next boundary.
    #[cfg(test)]
    pub(crate) fn grow(&self, receivers: usize) {
        self.offer(receivers);
        self.registry.boundary(false);
        self.registry.collect();
    }

    #[cfg(test)]
    pub(crate) fn status(&self) -> impl serde::Serialize {
        self.registry.status()
    }
}

/// Receivers to size the watches for: every country slot's air object and
/// a history in each of the content's `regions`, the most an hour can want.
fn watches_for(slots: usize, regions: usize) -> usize {
    slots.saturating_mul(regions.saturating_add(1))
}

/// Notifications running now.
pub fn active() -> usize {
    WATCHES.registry.active()
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything. Counts the hour when it will copy every held history afresh.
pub fn boundary(invalidate: bool) {
    let installed = WATCHES.registry.boundary(invalidate);
    let cause = if invalidate {
        Some(0)
    } else if installed {
        Some(1)
    } else {
        // The last hour's jobs joined, so nothing else holds the opening.
        crate::extended::air_histories::opening()
            .withdrawn()
            .then_some(2)
    };
    if let Some(cause) = cause.filter(|_| cfg!(feature = "history_changes")) {
        RESTARTS[cause].fetch_add(1, Ordering::Relaxed);
    }
}

/// Off the game thread: size the watches for `slots` country slots and the
/// content's live strategic-region count, before the hooks are installed
/// and whenever either grows. With fixed, readable rosters, visiting more of
/// the known regions then installs no larger table; a roster that grows,
/// an unreadable roster or dead receivers accumulating can still install
/// one, which restarts the epoch and copies every held history afresh.
pub fn presize(slots: usize, regions: usize) {
    WATCHES.offer(watches_for(slots, regions));
}

/// Off the game thread: size the watches for an hour's `receivers`, every
/// country air object and a history in each wanted region.
pub(crate) fn observe(receivers: usize) {
    WATCHES.offer(receivers);
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.registry.collect();
}

/// What a capture sees of the notifications.
pub trait Changes {
    /// `(epoch, revision)` when observation is live and no writer is running;
    /// equal values across hours prove no history changed.
    fn quiet(&self) -> Option<(u64, u64)>;
    /// `(epoch, sequence)` of one receiver, registering it; `None` when it
    /// cannot be observed.
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)>;
    /// `(epoch, calls)` of one writer site ([`MUTATIONS`] index); equal
    /// values across hours prove none of its receivers changed.
    fn site(&self, index: usize) -> Option<(u64, u64)>;
}

impl Changes for Watches {
    fn quiet(&self) -> Option<(u64, u64)> {
        self.registry.quiet()
    }
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        self.registry.sequence(receiver)
    }
    fn site(&self, index: usize) -> Option<(u64, u64)> {
        let (epoch, _) = self.registry.quiet()?;
        let calls = self.calls[index].load(Ordering::SeqCst);
        // A writer begun before this load and still running is refused.
        (self.registry.quiet()?.0 == epoch).then_some((epoch, calls))
    }
}

/// The hooks' domain.
pub struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        WATCHES.quiet()
    }
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(receiver)
    }
    fn site(&self, index: usize) -> Option<(u64, u64)> {
        WATCHES.site(index)
    }
}

/// Nothing observed: every history is read afresh.
pub struct Unobserved;
impl Changes for Unobserved {
    fn quiet(&self) -> Option<(u64, u64)> {
        None
    }
    fn sequence(&self, _: usize) -> Option<(u64, u64)> {
        None
    }
    fn site(&self, _: usize) -> Option<(u64, u64)> {
        None
    }
}

/// One capture's work.
pub fn record(counts: &crate::extended::air_histories::Counts) {
    for (target, value) in READS.iter().zip([
        counts.reused,
        counts.read,
        counts.advanced,
        counts.refreshed,
        u64::from(counts.quiet),
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
}

/// A verify read disagreed with the followed histories: reuse is disabled
/// for the attachment.
pub fn mismatch() {
    READS[5].fetch_add(1, Ordering::Relaxed);
    WATCHES.registry.poison();
}

pub fn status() -> impl serde::Serialize {
    let event_counts = WATCHES
        .calls
        .each_ref()
        .map(|value| value.load(Ordering::Relaxed));
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .zip(event_counts)
            .map(|(site, n)| (site.name, n)),
    );
    let reads = READS.each_ref().map(|value| value.load(Ordering::Relaxed));
    let restarts = RESTARTS
        .each_ref()
        .map(|value| value.load(Ordering::Relaxed));
    crate::status::status!({
        "configured": cfg!(feature = "history_changes"),
        "verify": cfg!(feature = "history_verify"),
        "poisoned": WATCHES.registry.poisoned(),
        "reused_countries": reads[0],
        "read_countries": reads[1],
        "advanced_histories": reads[2],
        "refreshed_histories": reads[3],
        "quiet_hours": reads[4],
        "mismatches": reads[5],
        "events": events,
        "watches": WATCHES.registry.status(),
        "restarts": {
            "invalidation": restarts[0],
            "table_install": restarts[1],
            "failed_hour": restarts[2],
        },
        "scope": "cumulative since attach; a quiet hour reuses every country's histories and an observed advance is applied without a read; a restart is an hour that copied every held history afresh: after a load, gap, recovery or attach, a larger watch table, or an hour whose part or AirMissions job failed or that was dropped before decoding"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "history_changes") {
            let callbacks = [
                region_history as *const () as usize,
                advance as *const () as usize,
            ];
            for (index, site) in MUTATIONS.iter().enumerate() {
                if engine::read::<[u8; 16]>(memory, base + site.rva) != Some(site.sentinel) {
                    return Err("air history hook sentinel differs");
                }
                let hook = unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }
                    .map_err(|_| "native hook construction failed")?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            WATCHES.registry.set_enabled(false);
        }
        for hook in &mut self.hooks {
            if !hook
                .set_enabled(enabled)
                .map_err(|_| "native hook state change failed")?
            {
                return Ok(false);
            }
        }
        WATCHES
            .registry
            .set_enabled(enabled && !self.hooks.is_empty());
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // SAFETY: like each Hook's relay, the watch tables are dropped only
        // once the hooks were never enabled, or were removed and drained
        // after the last capture.
        unsafe { WATCHES.registry.release() };
    }
}

unsafe extern "system" fn region_history(air: usize, region: i32) -> usize {
    let _notification = WATCHES.notify(air, 0);
    let original: unsafe extern "system" fn(usize, i32) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[0].load(Ordering::Acquire)) };
    unsafe { original(air, region) }
}
/// The receiver is `CLoopHistory+8`; the history itself is registered.
unsafe extern "system" fn advance(windows: usize) {
    let _notification = WATCHES.notify(windows.wrapping_sub(8), 1);
    let original: unsafe extern "system" fn(usize) =
        unsafe { std::mem::transmute(ORIGINAL[1].load(Ordering::Acquire)) };
    unsafe { original(windows) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_site_certificate_moves_with_its_calls_and_refuses_a_running_writer() {
        let watches = Watches::enabled(1, 1);
        let before = watches.site(1).unwrap();
        drop(watches.notify(0x40, 0));
        assert_eq!(watches.site(1), Some(before));
        let running = watches.notify(0x40, 1);
        assert_eq!(watches.site(1), None);
        drop(running);
        assert_eq!(watches.site(1), Some((before.0, before.1 + 1)));
        // A clear starts an epoch: no site certificate survives it.
        watches.registry.boundary(true);
        assert_ne!(watches.site(1).unwrap().0, before.0);
    }
}
