//! UnitLeaderTraits rows held per leader between hours. A leader's assigned Trait vector
//! (`cunit_leader::TRAIT_DATA`, with its count) is written at run time only by trait insertion and
//! removal, which updates the vector count. Each then recalculates modifiers on the same leader,
//! with no return or branch past the call; recalculation ends in a jump to its shared tail. The
//! naval tree family owns that tail's patch and the leader construction, copy construction and
//! destruction patches, and forwards each call Here  ([`forwarded`]), which notifies this family's
//! watch registry for the leader; it never changes game behaviour. A write therefore moves the
//! leader's certificate before the hour that could read it ends, and a reused address never keeps a
//! freed leader's rows. Loads write the vector directly and invalidate the hour, which revokes
//! every certificate. The trait definitions a row names (token id, ordinal, token) are written only
//! while the content database loads.
//!
//! The traits job reads a leader's vector only when no held read carries its
//! certificate ([`crate::extended::characters`]); every hour also reads the
//! carryable leaders of its sampled slice afresh and compares them, and a
//! difference poisons reuse for the attachment. `unit_leader_trait_verify`,
//! on by default during burn-in, reads every carryable leader.
use crate::{
    unit_leader_changes::{Changes, Counts},
    watch_registry::{Notification, WatchRegistry},
};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

static WATCHES: WatchRegistry = WatchRegistry::new();
/// Forwarded calls per [`Site`].
static CALLS: [AtomicU64; Site::ALL.len()] = [const { AtomicU64::new(0) }; Site::ALL.len()];
/// Reused, read, verified and mismatched leaders, and carrying hours.
static READS: [AtomicU64; 5] = [const { AtomicU64::new(0) }; 5];
static SAMPLED: crate::sampled_verify::Sampled = crate::sampled_verify::Sampled::burned_in(&[
    "carried UnitLeaderTraits rows differ from a fresh read",
]);
/// Leaders the latest capture visited, for off-thread sizing.
static LEADERS: AtomicUsize = AtomicUsize::new(0);

/// The naval tree family's forwarded leader sites.
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

    fn name(self) -> &'static str {
        match self {
            Self::RecalculateModifiers => "recalculate_modifiers",
            Self::Construct => "construct",
            Self::CopyConstruct => "copy_construct",
            Self::Destroy => "destroy",
        }
    }
}

/// Whether the traits job reads every carryable leader afresh and compares,
/// rather than the hour's slice.
pub const VERIFY: bool = cfg!(feature = "unit_leader_trait_verify");

/// The forwarded writers' domain.
pub struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        WATCHES.quiet()
    }
    fn sequence(&self, leader: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(leader)
    }
    fn mismatch(&self) {
        WATCHES.poison();
    }
}

/// Notifications running now.
pub fn active() -> usize {
    WATCHES.active()
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything.
pub fn boundary(invalidate: bool) {
    WATCHES.boundary(invalidate);
}

/// Off the game thread: size the watches for the leaders the latest capture
/// visited.
pub fn presize() {
    let leaders = LEADERS.load(Ordering::Relaxed);
    if leaders != 0 {
        WATCHES.offer(leaders);
    }
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.collect();
}

/// The naval tree family's leader patches enabled or disabled; outside
/// attach and detach only. Without them nothing is observed.
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

/// Hook path, allocation-free: a forwarded writer is about to change, build
/// or free `leader`. Hold the result over the native call.
pub fn forwarded(leader: usize, site: Site) -> Notification<'static> {
    CALLS[site as usize].fetch_add(1, Ordering::Relaxed);
    WATCHES.notify(leader)
}

/// The decode worker publishes one capture's counts and the leaders it
/// visited.
pub fn record(counts: &Counts, leaders: usize) {
    for (target, value) in READS.iter().zip([
        counts.reused,
        counts.read,
        counts.verified,
        counts.mismatches,
        u64::from(counts.reused != 0),
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    SAMPLED.record(counts.verified, counts.mismatches, 0);
    LEADERS.store(leaders, Ordering::Relaxed);
}

pub fn status() -> impl serde::Serialize {
    let events = crate::status::map(
        Site::ALL
            .iter()
            .zip(&CALLS)
            .map(|(site, calls)| (site.name(), calls.load(Ordering::Relaxed))),
    );
    let reads = READS.each_ref().map(|value| value.load(Ordering::Relaxed));
    crate::status::status!({
        "configured": cfg!(feature = "unit_leader_trait_changes"),
        "verify": VERIFY,
        "poisoned": WATCHES.poisoned(),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "reused_leaders": reads[0],
        "read_leaders": reads[1],
        "verified_leaders": reads[2],
        "mismatches": reads[3],
        "carrying_hours": reads[4],
        "events": events,
        "watches": WATCHES.status(),
        "scope": "cumulative since attach; a leader's UnitLeaderTraits rows are carried, without reading the leader, while the role census names it at the held address for the held character and no forwarded writer (events: the naval tree family's modifier recalculation tail and leader construct, copy_construct and destroy) touched it since the held read; every other leader is read and its rows held; every hour also reads the carryable leaders of its slice afresh (one in rotation_hours by leader id; every one in verify mode), publishes the fresh rows and disables reuse at the first mismatch (sampled_*)"
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each forwarded site counts its call.
    #[test]
    fn forwarded_sites_count_their_calls() {
        for site in Site::ALL {
            let before = CALLS[site as usize].load(Ordering::Relaxed);
            drop(forwarded(0x3000, site));
            assert_eq!(CALLS[site as usize].load(Ordering::Relaxed), before + 1);
        }
    }
}
