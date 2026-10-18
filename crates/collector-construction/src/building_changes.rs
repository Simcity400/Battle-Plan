//! Allocation-free notifications from the native writers of every building status's StateBuildings
//! and ProvinceBuildings rows ([`engine_mutations::building_mutations`]). A detour only notifies the shared
//! watch registry ([`WatchRegistry`]) for the `CBuildingStatus` its first argument names, counts
//! its site and calls the original; it never changes game behaviour. A building writer names its
//! status through the building's guarded backlink (`engine_native::cbuilding::BUILDING_STATUS`); when that
//! read fails the call revokes every certificate. Entering nudge mode (the developer map editor,
//! [`Receiver::Structure`]) poisons the domain: the nudger then rebuilds statuses and locations at
//! any moment without a hooked writer. Anything unobserved (disabled, poisoned, a refused
//! registration, an epoch change, a running writer) makes the capture read that status afresh.
//! Every notification of a registered status also marks it ([`Changes::take`]), which lets a
//! settled hour read only the statuses writers touched without walking the locations. This family
//! owns these patches, so each detour also forwards its call ([`forward`]) to
//! [`crate::territory_changes`], whose carried state and province rows read the same buildings, to
//! [`crate::rail_changes`], whose carried rail infos read the rail building of their province's
//! status, and to [`crate::raid_changes`], whose raid state inputs read a state's infrastructure
//! level.
use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::{building_mutations::MUTATIONS, building_mutations::Receiver};
use engine_native::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// Reused, read, verified and mismatched statuses; reused and read rows;
/// empty statuses; quiet hours; statuses read because a writer touched
/// them; settled hours.
static READS: [AtomicU64; 10] = [const { AtomicU64::new(0) }; 10];
/// The family's sampled comparisons.
static SAMPLED: crate::sampled_verify::Sampled = crate::sampled_verify::Sampled::burned_in(&[
    "carried StateBuildings or ProvinceBuildings rows differ from a fresh read",
    "a settled hour's sampled building status no longer reads as its held read",
]);
/// Statuses the latest complete capture walked, for off-thread sizing.
static STATUSES: AtomicUsize = AtomicUsize::new(0);

/// One observation domain: the watched statuses and each writer site's calls.
pub struct Watches {
    registry: WatchRegistry,
    calls: [AtomicU64; SITES],
    unnamed: AtomicU64,
    /// Calls of a [`Receiver::Structure`] site, each of which poisoned.
    structure: AtomicU64,
}

impl Default for Watches {
    fn default() -> Self {
        Self::new()
    }
}

impl Watches {
    pub const fn new() -> Self {
        Self {
            registry: WatchRegistry::marking(),
            calls: [const { AtomicU64::new(0) }; SITES],
            unnamed: AtomicU64::new(0),
            structure: AtomicU64::new(0),
        }
    }

    /// Hook path, allocation-free: a writer of `site` ([`MUTATIONS`] index)
    /// is about to change `status`; `None` names no status and revokes every
    /// certificate. Dropping the guard notifies its return.
    pub fn notify(&self, status: Option<usize>, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        match status {
            Some(status) => self.registry.notify(status),
            None => {
                self.unnamed.fetch_add(1, Ordering::Relaxed);
                self.registry.notify_every()
            }
        }
    }

    /// Hook path, allocation-free: `site` opens a path that may later
    /// rewrite any status and what locates it without a hooked writer (nudge
    /// mode). Reuse ends for the attachment, and every certificate taken
    /// before or while it runs is revoked.
    pub fn structure(&self, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.structure.fetch_add(1, Ordering::Relaxed);
        self.registry.poison();
        self.registry.notify_every()
    }

    /// Hook path: the notification of a call of `site` whose first argument
    /// is `first`, as the site's [`Receiver`] says.
    fn notify_site(&self, site: usize, first: usize) -> Notification<'_> {
        match MUTATIONS[site].1 {
            Receiver::Structure => self.structure(site),
            receiver => self.notify(status_of(receiver, first), site),
        }
    }

    /// An enabled domain with a table for `statuses` receivers installed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(statuses: usize) -> Self {
        let watches = Self::new();
        watches.registry.offer(statuses);
        watches.registry.boundary(false);
        watches.registry.set_enabled(true);
        watches
    }

    /// The hourly boundary; `invalidate` starts an epoch.
    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        self.registry.boundary(invalidate);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self) -> bool {
        self.registry.poisoned()
    }

    /// Registrations since the table was last cleared or installed.
    #[cfg(any(test, feature = "test-support"))]
    pub fn registered(&self) -> usize {
        self.registry.registered()
    }
}

/// What a capture sees of the notifications.
pub trait Changes: Sync {
    /// The single consumer of the marks: visits every registered status a
    /// writer touched since the previous take and returns the epoch they
    /// were marked in; `None` when that certifies nothing (unobserved, a
    /// writer runs or the epoch moved). A status registered in that epoch
    /// and not visited since was touched by no observed writer.
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64>;
    /// `(epoch, sequence)` of one status, registering it; `None` when it
    /// cannot be observed. Equal values across hours prove that no observed
    /// writer touched it.
    fn sequence(&self, status: usize) -> Option<(u64, u64)>;
    /// A fresh read disagreed with carried rows: disable reuse for the
    /// attachment.
    fn mismatch(&self);
}

impl Changes for Watches {
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        self.registry.take(visit)
    }
    fn sequence(&self, status: usize) -> Option<(u64, u64)> {
        self.registry.sequence(status)
    }
    fn mismatch(&self) {
        self.registry.poison();
    }
}

/// The hooks' domain.
pub struct Observer;
impl Changes for Observer {
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        WATCHES.take(visit)
    }
    fn sequence(&self, status: usize) -> Option<(u64, u64)> {
        WATCHES.sequence(status)
    }
    fn mismatch(&self) {
        WATCHES.mismatch();
    }
}

/// Whether captures read every carryable status afresh and compare, rather
/// than the hour's sampled slice ([`crate::sampled_verify`]).
pub const VERIFY: bool = cfg!(feature = "building_verify");

/// The status a site's first argument names: the argument itself, or a
/// building's guarded backlink; `None` when that does not read, and for a
/// site that names no status.
fn status_of(receiver: Receiver, first: usize) -> Option<usize> {
    match receiver {
        Receiver::Status => Some(first),
        Receiver::Building => {
            let memory = unsafe { engine_native::memory::LiveMemory::new() };
            first
                .checked_add(engine_native::cbuilding::BUILDING_STATUS)
                .and_then(|address| engine_native::read::<u64>(&memory, address))
                .and_then(|status| usize::try_from(status).ok())
                .filter(|&status| status != 0)
        }
        Receiver::Structure => None,
    }
}

/// The marks a forwarded building write holds over the native call.
#[must_use]
struct Forwarded(
    #[allow(dead_code)] Option<crate::territory_changes::Marks<'static>>,
    #[allow(dead_code)] Option<crate::dirty_set::Dirty<'static>>,
    #[allow(dead_code)] Option<crate::dirty_set::Dirty<'static>>,
);

/// Hook path, allocation-free: this family owns the building writers' patches, so it forwards each
/// call to [`crate::territory_changes`], whose The two state infrastructure terms and province fort
/// sums read a status's buildings, to [`crate::rail_changes`], whose rail infos read the rail
/// building of their province's status, and to [`crate::raid_changes`], whose state inputs read a
/// state's infrastructure level. A named status marks its province or state through its own flags
/// and location id (`engine_native::cbuilding_status::FLAGS` and `ID`, written only by the status
/// constructor); a status that cannot be named or read marks every province and state; entering
/// nudge mode poisons the three families. The marks cover the original in flight.
fn forward(receiver: Receiver, first: usize) -> Forwarded {
    if receiver == Receiver::Structure {
        crate::territory_changes::poison();
        crate::rail_changes::poison();
        crate::raid_changes::poison();
        return Forwarded(None, None, None);
    }
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let location = status_of(receiver, first)
        .and_then(|status| status.checked_add(engine_native::cbuilding_status::FLAGS))
        .and_then(|address| engine_native::read::<[u8; 8]>(&memory, address));
    match location {
        Some(bytes) => {
            let flags = u32::from_le_bytes(bytes[..4].try_into().unwrap());
            let location = u32::from_le_bytes(bytes[4..].try_into().unwrap());
            Forwarded(
                Some(crate::territory_changes::building_changed(flags, location)),
                crate::rail_changes::building_changed(flags, location),
                crate::raid_changes::building_changed(flags, location),
            )
        }
        None => Forwarded(
            Some(crate::territory_changes::building_unnamed()),
            Some(crate::rail_changes::building_unnamed()),
            Some(crate::raid_changes::building_unnamed()),
        ),
    }
}

/// Notifications running now.
pub fn active() -> usize {
    WATCHES.registry.active()
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything.
pub fn boundary(invalidate: bool) {
    WATCHES.registry.boundary(invalidate);
}

/// Off the game thread: size the watches for the statuses the latest capture
/// walked. Nothing is offered before a capture walked them: a later larger
/// table waits for an invalidating hour, while the first installs at once.
pub fn presize() {
    let statuses = STATUSES.load(Ordering::Relaxed);
    if statuses != 0 {
        WATCHES.registry.offer(statuses);
    }
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.registry.collect();
}

/// One capture's statuses and rows. `empty` statuses have no building and
/// are neither read nor reused, unless a writer touched them. `settled`
/// marks a capture that carried both tables without walking the locations,
/// reading the `marked` statuses a writer touched (counted as read) and its
/// sampled slice; `quiet` a settled one that read no touched status;
/// `unread_slice` one whose settled slice found a status that no longer
/// reads as its held read (a changed header or a failed read), a mismatch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    pub verified: u64,
    pub mismatches: u64,
    pub reused_rows: u64,
    pub read_rows: u64,
    pub empty: u64,
    pub marked: u64,
    pub settled: bool,
    pub quiet: bool,
    pub unread_slice: bool,
}

/// A worker publishes one capture's counts and, when it walked every status,
/// their number for sizing.
pub fn record(counts: &Counts, statuses: Option<usize>) {
    for (target, value) in READS.iter().zip([
        counts.reused,
        counts.read,
        counts.verified,
        counts.mismatches,
        counts.reused_rows,
        counts.read_rows,
        counts.empty,
        u64::from(counts.quiet),
        counts.marked,
        u64::from(counts.settled),
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    SAMPLED.record(
        counts.verified,
        counts.mismatches,
        usize::from(counts.unread_slice),
    );
    if let Some(statuses) = statuses {
        STATUSES.store(statuses, Ordering::Relaxed);
    }
}

pub fn status() -> impl serde::Serialize {
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .zip(&WATCHES.calls)
            .map(|((site, _), n)| (site.name, n.load(Ordering::Relaxed))),
    );
    let reads = READS.each_ref().map(|value| value.load(Ordering::Relaxed));
    crate::status::status!({
        "configured": cfg!(feature = "building_changes"),
        "verify": VERIFY,
        "poisoned": WATCHES.registry.poisoned(),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "reused_statuses": reads[0],
        "read_statuses": reads[1],
        "verified_statuses": reads[2],
        "mismatches": reads[3],
        "reused_rows": reads[4],
        "read_rows": reads[5],
        "empty_statuses": reads[6],
        "quiet_hours": reads[7],
        "marked_statuses": reads[8],
        "settled_hours": reads[9],
        "events": events,
        "unnamed_receivers": WATCHES.unnamed.load(Ordering::Relaxed),
        "structure_poisoned": WATCHES.structure.load(Ordering::Relaxed) != 0,
        "watches": WATCHES.registry.status(),
        "scope": "cumulative since attach; a walk reads every location's identity and status headers and carries a state's or province's StateBuildings or ProvinceBuildings rows when its status, identity and vector headers are those of the held read and no hooked writer touched the status since; settled_hours counts hours that walked no location, because the latest walk certified every status (empty_statuses, without buildings, too), the watch epoch held and both location table headers are unchanged: they read afresh only the statuses a hooked writer marked since the previous hour (marked_statuses, from the status alone, its identity unchanged) and carry every other; quiet_hours counts the settled hours that marked none; every hour also reads the carryable statuses of its slice afresh (one in rotation_hours by scope and location id; every one in verify mode, which never settles), publishes the fresh rows and disables reuse at the first mismatch (sampled_*), and in a settled hour a sampled status that no longer reads as held is a mismatch; structure_poisoned: entering nudge mode (enter_nudge_mode) disabled reuse for the attachment"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    /// A differing sentinel leaves the family uninstalled; the building
    /// tables then read every status afresh.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "building_changes")
            && MUTATIONS.iter().all(|(site, _)| {
                engine_native::read::<[u8; 16]>(memory, base + site.rva) == Some(site.sentinel)
            })
        {
            let callbacks: [usize; SITES] = [
                set_level as *const () as usize,
                apply_damage_step as *const () as usize,
                advance_repair as *const () as usize,
                destroy_healthy_levels as *const () as usize,
                get_or_create_building as *const () as usize,
                read_building_status as *const () as usize,
                clear_building_status as *const () as usize,
                enter_nudge_mode as *const () as usize,
            ];
            for (index, (site, _)) in MUTATIONS.iter().enumerate() {
                let hook = unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }
                    .map_err(|_| "native hook construction failed")?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        if hooks.is_empty() {
            // Nothing forwards building writes: territory_changes cannot
            // certify a state's infrastructure or a province's fort sum, nor
            // rail_changes a rail building's health, nor raid_changes a
            // state's infrastructure level.
            crate::territory_changes::poison();
            crate::rail_changes::poison();
            crate::raid_changes::poison();
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            WATCHES.registry.set_enabled(false);
            collector_observers::nuclear_strikes::forwarder(
                replay_data::rows::NuclearStrikePart::Buildings,
                false,
            );
            // The rail and raid state domains observe only while these
            // forward their writes.
            crate::rail_changes::forwarder(false);
            crate::raid_changes::forwarder(false);
        }
        // One census and suspension for the whole family, shared with the
        // other families in a runtime round (`hook::shared`): a thread at any
        // of its sites, or in a tap, holds every site back until the next
        // attempt.
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "native hook state change failed")?
        {
            return Ok(false);
        }
        WATCHES
            .registry
            .set_enabled(enabled && !self.hooks.is_empty());
        collector_observers::nuclear_strikes::forwarder(
            replay_data::rows::NuclearStrikePart::Buildings,
            enabled && !self.hooks.is_empty(),
        );
        crate::rail_changes::forwarder(enabled && !self.hooks.is_empty());
        crate::raid_changes::forwarder(enabled && !self.hooks.is_empty());
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

// Each site takes integer or pointer arguments only (the asm prologues use
// RCX..R9 and the caller's stack slots, which a Win64 detour forwards) and
// returns RAX unchanged from its original; the first names the status as
// its [`Receiver`] says. The notification covers the original in flight.
macro_rules! callback {
    ($name:ident, $site:expr, ($first:ident $(, $arg:ident)*)) => {
        unsafe extern "system" fn $name($first: usize $(, $arg: usize)*) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::BuildingChanges);
            let _notification = WATCHES.notify_site($site, $first);
            let _forwarded = forward(MUTATIONS[$site].1, $first);
            let original: unsafe extern "system" fn($first: usize $(, $arg: usize)*) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original($first $(, $arg)*) })
        }
    };
}
// Set level (building, level, notify) -> bool.
callback!(set_level, 0, (building, level, notify));
// Apply damage step (building, out, amount, flag, cause, flag).
callback!(
    apply_damage_step,
    1,
    (building, out, amount, first, cause, second)
);
// Advance repair (building, out, progress, flag).
callback!(advance_repair, 2, (building, out, progress, flag));
// Destroy healthy levels (building, out, levels, flag, source):
// a strike's building damage, which the nuclear strike family observes
// through this patch ([`collector_observers::nuclear_strikes::building`]).
unsafe extern "system" fn destroy_healthy_levels(
    building: usize,
    out: usize,
    levels: usize,
    flag: usize,
    source: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::BuildingChanges);
    let _notification = WATCHES.notify_site(3, building);
    let _forwarded = forward(MUTATIONS[3].1, building);
    let _nuclear = collector_observers::nuclear_strikes::building(building, source);
    let original: unsafe extern "system" fn(usize, usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[3].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(building, out, levels, flag, source) })
}
// Get or create building (status, template).
callback!(get_or_create_building, 4, (status, template));
// Read building status (status, reader).
callback!(read_building_status, 5, (status, reader));
// Clear building status (status).
callback!(clear_building_status, 6, (status));
// Enter nudge mode (idler, a, b, c, d) -> idler. Its only caller, game idler update, passes five
// arguments (the fifth at its stack slot); a sixth forwards the next caller stack slot, which the
// original never reads.
callback!(enter_nudge_mode, 7, (idler, a, b, c, d, spare));

#[cfg(test)]
mod tests {
    use super::*;

    /// A building writer names the status its live backlink holds; one
    /// whose backlink is null names none, so it revokes every certificate.
    #[test]
    fn a_building_writer_names_its_backlinked_status() {
        let mut building = vec![0_u8; engine_native::cbuilding::TEMPLATE + 8];
        let status = 0x7654_3210_usize;
        let at = engine_native::cbuilding::BUILDING_STATUS;
        building[at..at + 8].copy_from_slice(&(status as u64).to_le_bytes());
        let address = building.as_ptr() as usize;
        assert_eq!(status_of(Receiver::Building, address), Some(status));
        assert_eq!(status_of(Receiver::Status, address), Some(address));
        building[at..at + 8].fill(0);
        assert_eq!(
            status_of(Receiver::Building, building.as_ptr() as usize),
            None
        );
        let watches = Watches::enabled(16);
        let certified = Changes::sequence(&watches, status).unwrap();
        drop(watches.notify(None, 0));
        assert_ne!(Changes::sequence(&watches, status), Some(certified));
        assert_eq!(watches.unnamed.load(Ordering::Relaxed), 1);
    }

    /// Entering nudge mode names no status: it poisons the domain, so no
    /// certificate is given and no take certifies again, and counts its call.
    #[test]
    fn entering_nudge_mode_poisons_the_domain() {
        let site = MUTATIONS
            .iter()
            .position(|&(_, receiver)| receiver == Receiver::Structure)
            .unwrap();
        assert_eq!(MUTATIONS[site].0.name, "enter_nudge_mode");
        assert_eq!(MUTATIONS[site].0.rva, 0x12b_ea10);
        let watches = Watches::enabled(16);
        let certified = Changes::sequence(&watches, 0x1000).unwrap();
        let taken = Changes::take(&watches, &mut |_| {}).unwrap();
        let running = watches.notify_site(site, 0x1000);
        assert!(watches.poisoned());
        assert_eq!(Changes::take(&watches, &mut |_| {}), None);
        drop(running);
        watches.boundary(false);
        assert_eq!(Changes::sequence(&watches, 0x1000), None);
        assert_eq!(Changes::take(&watches, &mut |_| {}), None);
        assert_ne!(Some(certified), Changes::sequence(&watches, 0x1000));
        assert_ne!(Some(taken), Changes::take(&watches, &mut |_| {}));
        assert_eq!(watches.calls[site].load(Ordering::Relaxed), 1);
        assert_eq!(watches.structure.load(Ordering::Relaxed), 1);
        assert_eq!(watches.unnamed.load(Ordering::Relaxed), 0);
    }
}
