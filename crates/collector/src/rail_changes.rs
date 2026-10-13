//! Allocation-free notifications from the native writers of every rail
//! info's RailCooldowns row, its building and selection inputs and its
//! RailEdges rows ([`engine::railway_mutations`]). A detour only marks the
//! province whose info its arguments name in the family's [`DirtySet`],
//! counts its site and calls the original; it never changes game behaviour.
//!
//! - Setting a province on cooldown and the info constructor name the
//!   province in EDX; setting railway and the info destructor name it through
//!   the info's definition id.
//! - The daily cooldown countdown writes the info of every occurrence on the
//!   manager's cooldown list: it marks [`COUNTDOWN`], and the consumer reads
//!   every province on the list it held and on this hour's list ([`crate::extended::naval_history`]).
//!   Province 0 never holds an info (the walk refuses one), so its bit is
//!   free for this.
//! - The info's and the manager's save readers and the manager's
//!   constructor and destructor name nothing and move the epoch.
//! - The rail building's health is written by the building status writers,
//!   whose patches [`crate::building_changes`] owns: it forwards each call
//!   ([`building_changed`], [`building_unnamed`]), and entering nudge mode
//!   or an uninstalled building family poisons this domain. The domain
//!   observes only while both its own hooks and that forwarder are enabled.
//!
//! Anything unobserved (disabled, poisoned, an id beyond the bitmap, an
//! epoch change, a running writer) makes the capture read every info.
use crate::{
    dirty_set::{Dirty, DirtySet},
    hook::Hook,
};
use engine::{
    Memory,
    railway_mutations::{MUTATIONS, Receiver},
};
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static WATCHES: Watches = Watches::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// Per [`Counts`] field, cumulative.
static READS: [AtomicU64; 12] = [const { AtomicU64::new(0) }; 12];
/// The family's sampled comparisons.
static SAMPLED: crate::sampled_verify::Sampled = crate::sampled_verify::Sampled::burned_in(&[
    "carried RailCooldowns rows, their inputs or RailEdges rows differ from a fresh read",
    "an unmarked rail info or cooldown list changed",
]);
/// Rail slots (province ids) of the latest complete capture, for off-thread
/// sizing.
static SLOTS: AtomicUsize = AtomicUsize::new(0);

/// The bit the daily countdown marks: province 0, which never holds an info.
pub(crate) const COUNTDOWN: usize = 0;

/// One observation domain: the marked provinces and each writer site's
/// calls.
pub(crate) struct Watches {
    set: DirtySet,
    calls: [AtomicU64; SITES],
    /// Receivers whose province did not read.
    unnamed: AtomicU64,
    /// Forwarded building writes of a province status, and those naming no
    /// status.
    buildings: [AtomicU64; 2],
    /// This family's hooks, and the building family's forwarder, enabled.
    hooks: AtomicBool,
    forwarder: AtomicBool,
}

impl Watches {
    pub(crate) const fn new() -> Self {
        Self {
            set: DirtySet::new(),
            calls: [const { AtomicU64::new(0) }; SITES],
            unnamed: AtomicU64::new(0),
            buildings: [const { AtomicU64::new(0) }; 2],
            hooks: AtomicBool::new(false),
            forwarder: AtomicBool::new(false),
        }
    }

    /// Hook path, allocation-free: a writer of `site` ([`MUTATIONS`] index)
    /// whose receiver `target` names is about to run. Dropping the guard
    /// marks its return.
    pub(crate) fn notify(&self, site: usize, target: Target) -> Dirty<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        match target {
            Target::Province(Some(id)) => self.set.notify(id),
            Target::Province(None) => {
                self.unnamed.fetch_add(1, Ordering::Relaxed);
                self.set.notify_every()
            }
            Target::Countdown => self.set.notify(COUNTDOWN),
            Target::Every => self.set.notify_every(),
        }
    }

    /// A forwarded building write of the status with `flags` and
    /// `location`: a province status marks its province; a state status
    /// holds no rail building.
    fn building(&self, flags: u32, location: u32) -> Option<Dirty<'_>> {
        if flags & engine::cbuilding_status::PROVINCE_FLAG == 0 {
            return None;
        }
        self.buildings[0].fetch_add(1, Ordering::Relaxed);
        Some(match location {
            0 => {
                self.unnamed.fetch_add(1, Ordering::Relaxed);
                self.set.notify_every()
            }
            id => self.set.notify(id as usize),
        })
    }

    fn building_unnamed(&self) -> Dirty<'_> {
        self.buildings[1].fetch_add(1, Ordering::Relaxed);
        self.set.notify_every()
    }

    /// Observe only while both the hooks and the forwarder are enabled;
    /// enabling starts an epoch.
    fn apply(&self) {
        self.set.set_enabled(
            self.hooks.load(Ordering::SeqCst) && self.forwarder.load(Ordering::SeqCst),
        );
    }

    /// An enabled domain with a bitmap for `ids` provinces installed.
    #[cfg(test)]
    pub(crate) fn enabled(ids: usize) -> Self {
        let watches = Self::new();
        watches.set.offer(ids);
        watches.set.boundary(false);
        watches.hooks.store(true, Ordering::SeqCst);
        watches.forwarder.store(true, Ordering::SeqCst);
        watches.apply();
        watches
    }

    /// The hourly boundary; `invalidate` starts an epoch.
    #[cfg(test)]
    pub(crate) fn boundary(&self, invalidate: bool) {
        self.set.boundary(invalidate);
    }

    /// The building family's forwarder turned off or on.
    #[cfg(test)]
    pub(crate) fn set_forwarder(&self, enabled: bool) {
        self.forwarder.store(enabled, Ordering::SeqCst);
        self.apply();
    }

    #[cfg(test)]
    pub(crate) fn poisoned(&self) -> bool {
        self.set.poisoned()
    }

    #[cfg(test)]
    pub(crate) fn mark_building(&self, flags: u32, location: u32) -> Option<Dirty<'_>> {
        self.building(flags, location)
    }
}

/// What a capture sees of the notifications.
pub(crate) trait Changes: Sync {
    /// The single consumer's hourly take of the marked provinces
    /// ([`DirtySet::take`]), [`COUNTDOWN`] included; `None` certifies
    /// nothing.
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64>;
    /// A fresh read disagreed with carried outputs: disable reuse for the
    /// attachment.
    fn mismatch(&self);
}

impl Changes for Watches {
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        self.set.take(visit)
    }
    fn mismatch(&self) {
        self.set.poison();
    }
}

/// The hooks' domain.
pub(crate) struct Observer;
impl Changes for Observer {
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        WATCHES.take(visit)
    }
    fn mismatch(&self) {
        WATCHES.mismatch();
    }
}

/// Whether captures read every batch afresh and compare each carryable
/// info, rather than the hour's sampled slice of batches
/// ([`crate::sampled_verify`]).
pub(crate) const VERIFY: bool = cfg!(feature = "rail_verify");

/// What a site's arguments name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Target {
    /// A province; `None` when it cannot be named.
    Province(Option<usize>),
    /// Every occurrence on the manager's cooldown list.
    Countdown,
    /// Infos it does not name, or the slot table itself.
    Every,
}

/// What `receiver` names given a site's first two arguments.
fn target(receiver: Receiver, first: usize, second: usize) -> Target {
    match receiver {
        // EDX alone: the native sign-extends it (`MOVSXD` in set province on cooldown).
        Receiver::Province => Target::Province(
            u32::try_from(second as u32 as i32)
                .ok()
                .map(|id| id as usize),
        ),
        Receiver::Info => {
            // SAFETY: a guarded read of live memory on the calling thread.
            let memory = unsafe { engine::memory::LiveMemory::new() };
            Target::Province(
                engine::railway_mutations::province(&memory, first).map(|id| id as usize),
            )
        }
        Receiver::CooldownList => Target::Countdown,
        Receiver::Every => Target::Every,
    }
}

/// Notifications running now.
pub fn active() -> usize {
    WATCHES.set.active()
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything.
pub fn boundary(invalidate: bool) {
    WATCHES.set.boundary(invalidate);
}

/// Disable reuse for the attachment: a writer this family cannot observe
/// may run (nudge mode, or no building family to forward building writes).
pub fn poison() {
    WATCHES.set.poison();
}

/// The building family's forwarder enabled or disabled; outside attach and
/// detach only.
pub fn forwarder(enabled: bool) {
    WATCHES.forwarder.store(enabled, Ordering::SeqCst);
    WATCHES.apply();
}

/// Hook path, allocation-free: a building writer of the status with `flags`
/// and `location` (`CBuildingStatus+0x68`/`+0x6C`) is about to run. Marks
/// the province of a province status, whose rail building the info at that
/// slot holds. Hold the result over the native call.
pub(crate) fn building_changed(flags: u32, location: u32) -> Option<Dirty<'static>> {
    WATCHES.building(flags, location)
}

/// Hook path, allocation-free: a building writer whose status cannot be
/// named is about to run; every info is read afresh.
pub(crate) fn building_unnamed() -> Dirty<'static> {
    WATCHES.building_unnamed()
}

/// Off the game thread: size the bitmap for the slots the latest capture
/// walked. Nothing is offered before a capture walked them: a later larger
/// bitmap waits for an invalidating hour, while the first installs at once.
pub fn presize() {
    let slots = SLOTS.load(Ordering::Relaxed);
    if slots != 0 {
        WATCHES.set.offer(slots);
    }
}

/// Off the game thread: free a replaced bitmap.
pub fn collect() {
    WATCHES.set.collect();
}

/// One capture's infos, edge rows and batches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub reused: u64,
    pub read: u64,
    pub verified: u64,
    pub mismatches: u64,
    pub reused_edges: u64,
    pub read_edges: u64,
    /// Batches published from held outputs without a read.
    pub carried_batches: u64,
    /// Batches read.
    pub read_batches: u64,
    /// Provinces the take marked, with those of the countdown's lists.
    pub marked: u64,
    /// The take saw the daily countdown.
    pub countdown: bool,
    /// No held batch could be carried: everything was read.
    pub full: bool,
    /// An unmarked info or cooldown list changed (counted in `mismatches`).
    pub unmarked: bool,
}

/// A worker publishes one capture's counts and, when it completed, the rail
/// slots it walked for sizing.
pub(crate) fn record(counts: &Counts, slots: Option<usize>) {
    for (target, value) in READS.iter().zip([
        counts.reused,
        counts.read,
        counts.verified,
        counts.mismatches,
        counts.reused_edges,
        counts.read_edges,
        counts.carried_batches,
        counts.read_batches,
        counts.marked,
        u64::from(counts.countdown),
        u64::from(counts.full),
        u64::from(counts.marked == 0 && !counts.countdown && counts.carried_batches != 0),
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    SAMPLED.record(
        counts.verified,
        counts.mismatches,
        usize::from(counts.unmarked),
    );
    if let Some(slots) = slots {
        SLOTS.store(slots, Ordering::Relaxed);
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
        "configured": cfg!(feature = "rail_changes"),
        "verify": VERIFY,
        "poisoned": WATCHES.set.poisoned(),
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "reused_infos": reads[0],
        "read_infos": reads[1],
        "verified_infos": reads[2],
        "mismatches": reads[3],
        "reused_edges": reads[4],
        "read_edges": reads[5],
        "carried_batches": reads[6],
        "read_batches": reads[7],
        "marked_provinces": reads[8],
        "countdown_hours": reads[9],
        "full_reads": reads[10],
        "quiet_hours": reads[11],
        "events": events,
        "unnamed_receivers": WATCHES.unnamed.load(Ordering::Relaxed),
        "building_writes": WATCHES.buildings[0].load(Ordering::Relaxed),
        "unnamed_building_writes": WATCHES.buildings[1].load(Ordering::Relaxed),
        "set": WATCHES.set.status(),
        "scope": "cumulative since attach; a batch of 64 rail slots is published from the held outputs of its last complete read (cooldown rows, building and selection inputs, indexed levels, RailEdges rows) without reading any slot when the hour's take returned the held capture's epoch, no hooked writer marked any of its provinces and the manager, rail table and province table headers are unchanged; the daily countdown (countdown_hours) marks every province on the held and the current cooldown lists, which are read every hour; in a batch read for a marked province an unmarked info whose identity, cooldown and pair header are those of its held read is carried; every hour also reads its slice of batches afresh (one in rotation_hours by batch; every one in verify mode), publishes the fresh outputs and disables reuse at the first mismatch, as does an unmarked change (sampled_*); quiet_hours had no marked province or countdown and carried every batch but the slice's"
    })
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    /// A differing sentinel leaves the family uninstalled; the rail tables
    /// then read every info afresh.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "rail_changes")
            && MUTATIONS.iter().all(|(site, _)| {
                engine::read::<[u8; 16]>(memory, base + site.rva) == Some(site.sentinel)
            })
        {
            let callbacks: [usize; SITES] = [
                set_province_on_cooldown as *const () as usize,
                update_cooldowns as *const () as usize,
                set_railway as *const () as usize,
                construct_info as *const () as usize,
                destroy_info as *const () as usize,
                read_info as *const () as usize,
                construct_manager as *const () as usize,
                destroy_manager as *const () as usize,
                read_manager as *const () as usize,
            ];
            for (index, (site, _)) in MUTATIONS.iter().enumerate() {
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
            WATCHES.hooks.store(false, Ordering::SeqCst);
            WATCHES.apply();
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
            .hooks
            .store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        WATCHES.apply();
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // SAFETY: like each Hook's relay, the bitmap is dropped only once the
        // hooks were never enabled, or were removed and drained after the
        // last capture; the building family's forwarders are restored with
        // the other families before any hook is dropped.
        unsafe { WATCHES.set.release() };
    }
}

// Each site takes at most four integer or pointer arguments (RCX..R9) and
// returns RAX unchanged from its original; the first two name the province
// as its [`Receiver`] says. The mark covers the original in flight.
macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::RailChanges);
            let _mark = WATCHES.notify($site, target(MUTATIONS[$site].1, first, second));
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(first, second, third, fourth) })
        }
    };
}
// Set province on cooldown (manager, province, ticks).
callback!(set_province_on_cooldown, 0);
// Update cooldowns (manager), daily.
callback!(update_cooldowns, 1);
// Set railway (info, neighbour, level).
callback!(set_railway, 2);
// Rail-info construction(info, province id).
callback!(construct_info, 3);
// Rail-info scalar deleting destructor(info, flags).
callback!(destroy_info, 4);
// Read info (info, reader, token).
callback!(read_info, 5);
// Railway manager construction(manager).
callback!(construct_manager, 6);
// Railway manager scalar deleting destructor(manager, flags).
callback!(destroy_manager, 7);
// Read manager (manager, reader, token).
callback!(read_manager, 8);

#[cfg(test)]
mod tests {
    use super::*;

    fn take(watches: &Watches) -> (Option<u64>, Vec<usize>) {
        let mut ids = Vec::new();
        let epoch = watches.take(&mut |id| ids.push(id));
        (epoch, ids)
    }

    fn site(name: &str) -> usize {
        MUTATIONS
            .iter()
            .position(|(site, _)| site.name == name)
            .unwrap()
    }

    /// The setter and the constructor name their province in EDX alone; an
    /// info names its province through its definition's id; the countdown
    /// names its list and the rest nothing.
    #[test]
    fn receivers_name_their_province() {
        assert_eq!(
            target(Receiver::Province, 0, 0xdead_0000_0000_0207),
            Target::Province(Some(0x207))
        );
        // A negative EDX names none.
        assert_eq!(
            target(Receiver::Province, 0, u32::MAX as usize),
            Target::Province(None)
        );
        let mut info = vec![0_u8; 0x100];
        let mut definition = vec![0_u8; 0x100];
        let id = engine::province_definition::ID;
        definition[id..id + 4].copy_from_slice(&517_u32.to_le_bytes());
        let at = engine::cprovince_railway_info::PROVINCE_RECORD;
        info[at..at + 8].copy_from_slice(&(definition.as_ptr() as u64).to_le_bytes());
        assert_eq!(
            target(Receiver::Info, info.as_ptr() as usize, 7),
            Target::Province(Some(517))
        );
        info[at..at + 8].copy_from_slice(&0_u64.to_le_bytes());
        assert_eq!(
            target(Receiver::Info, info.as_ptr() as usize, 7),
            Target::Province(None)
        );
        assert_eq!(target(Receiver::CooldownList, 0, 0), Target::Countdown);
        assert_eq!(target(Receiver::Every, 0, 0), Target::Every);
        drop(definition);
    }

    /// A named writer marks its province, the countdown marks its bit, and
    /// a writer naming nothing (or whose province does not read) moves the
    /// epoch, so the consumer reads every info.
    #[test]
    fn each_site_marks_its_province_or_moves_the_epoch() {
        let watches = Watches::enabled(1_024);
        let (Some(epoch), _) = take(&watches) else {
            unreachable!("observing")
        };
        drop(watches.notify(site("set_province_on_cooldown"), Target::Province(Some(70))));
        drop(watches.notify(site("set_railway"), Target::Province(Some(3))));
        drop(watches.notify(site("update_cooldowns"), Target::Countdown));
        assert_eq!(take(&watches), (Some(epoch), vec![COUNTDOWN, 3, 70]));
        drop(watches.notify(site("set_railway"), Target::Province(None)));
        assert_ne!(take(&watches).0, Some(epoch));
        assert_eq!(watches.unnamed.load(Ordering::Relaxed), 1);
        let (Some(epoch), _) = take(&watches) else {
            unreachable!("observing")
        };
        for name in [
            "read_info",
            "construct_manager",
            "destroy_manager",
            "read_manager",
        ] {
            drop(watches.notify(site(name), Target::Every));
            assert_ne!(take(&watches).0, Some(epoch), "{name}");
        }
        assert_eq!(
            watches.calls[site("set_railway")].load(Ordering::Relaxed),
            2
        );
    }

    /// A forwarded write of a province status marks that province; a state
    /// status marks nothing; a status without a location moves the epoch.
    #[test]
    fn forwarded_building_writes_mark_province_statuses() {
        use engine::cbuilding_status::{PROVINCE_FLAG, STATE_FLAG};
        let watches = Watches::enabled(1_024);
        let (Some(epoch), _) = take(&watches) else {
            unreachable!("observing")
        };
        drop(watches.mark_building(PROVINCE_FLAG, 300));
        assert!(watches.mark_building(STATE_FLAG, 12).is_none());
        assert_eq!(take(&watches), (Some(epoch), vec![300]));
        drop(watches.mark_building(PROVINCE_FLAG, 0));
        assert_ne!(take(&watches).0, Some(epoch));
        let (Some(epoch), _) = take(&watches) else {
            unreachable!("observing")
        };
        drop(watches.building_unnamed());
        assert_ne!(take(&watches).0, Some(epoch));
    }

    /// Without the building family's forwarder nothing certifies: a building
    /// write could go unseen. Its return starts an epoch.
    #[test]
    fn the_domain_observes_only_with_the_forwarder() {
        let watches = Watches::enabled(64);
        let (Some(epoch), _) = take(&watches) else {
            unreachable!("observing")
        };
        watches.set_forwarder(false);
        assert_eq!(take(&watches).0, None);
        watches.set_forwarder(true);
        assert_ne!(take(&watches).0, Some(epoch));
        watches.mismatch();
        assert!(watches.poisoned());
        assert_eq!(take(&watches).0, None);
    }
}
