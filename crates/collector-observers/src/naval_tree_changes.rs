//! Allocation-free notifications from the native writers of the two naval hover trees
//! NavalModifierTrees carries: a country's modifier tree (`ccountry::MODIFIER_ACCUMULATOR`) and a
//! commander's ([`engine_mutations::naval_tree_mutations`]). A detour only notifies the shared watch registry
//! ([`WatchRegistry`]) for the CCountry or CUnitLeader its first argument names, counts its site
//! and calls the original; it never changes game behaviour. This observer patches the commander
//! sites: the two country sites belong to families that already patch them and forward Each call
//! ([`forwarded`]): country modifier recalculation to country_input_changes, country appearance
//! reset to politics. The family observes only while its own patches and both forwarders are
//! enabled ([`forwarder`]). Receivers are the countries and commanders a naval tree capture
//! registered. Anything unobserved (disabled, poisoned, a refused registration, an epoch change, a
//! running writer) makes the capture read that tree afresh. The commander sites are also the only
//! writers of the retreat bans' blocks, so they forward each call to
//! [`crate::retreat_ban_changes`]; the leader constructors and destructor forward theirs to
//! [`crate::unit_leader_changes`].
//! Its hook lifecycle also owns [`crate::occupation_changes`], whose separate certificates
//! receive these same country writers and the state, relation and exile forwarders.
//!
//! The same certificates key the naval mission country and commander rows
//! ([`crate::extended::naval_mission_inputs`]): a country whose own and
//! whose leaders' certificates moved is read within the hour ([`missions`]).
//!
//! The held trees live in a [`Store`] that the naval combat part takes at the
//! start of its capture and returns at its end; a capture that does not
//! complete forgets it.
use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::{naval_tree_mutations::COUNTRY_SITES, naval_tree_mutations::MUTATIONS};
use engine_native::Memory;
use engine_naval_history::naval_modifier_trees::RawNode;
use std::{
    collections::HashMap,
    ops::Range,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

const SITES: usize = MUTATIONS.len();
/// The commander sites this family patches; the country sites before them
/// are forwarded.
const PATCHED_SITES: usize = SITES - COUNTRY_SITES;
static WATCHES: WatchRegistry = WatchRegistry::new();
static CALLS: [AtomicU64; SITES] = [const { AtomicU64::new(0) }; SITES];
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// Reused and read trees; read trees of the kinds that are never carried;
/// failed captures.
static READS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
/// Country and commander receivers the latest capture reached, for
/// off-thread sizing: the watches must exist before any tree is held.
static RECEIVERS: AtomicUsize = AtomicUsize::new(0);
/// Country and fleet-leader receivers the latest naval mission read keyed.
static MISSION_RECEIVERS: AtomicUsize = AtomicUsize::new(0);
/// Country and fleet-leader receivers the ShipModels task-force terms can
/// key (two per task force of the largest census): those certificates
/// carry its doctrine, weather and finished rows, on saves without a naval
/// combat or mission to size the watches too.
static SHIP_MODEL_RECEIVERS: AtomicUsize = AtomicUsize::new(0);
/// Countries and commanders the latest ReserveModifierTrees capture keyed
/// (collector-combat), whose certificates carry their trees.
static DIVISION_RECEIVERS: AtomicUsize = AtomicUsize::new(0);
static RESERVE_RECEIVERS: AtomicUsize = AtomicUsize::new(0);
/// Countries the latest RaidIntelInputs walk keyed
/// ([`crate::extended::raid_intel_carry`]).
static RAID_INTEL_RECEIVERS: AtomicUsize = AtomicUsize::new(0);
/// The counter walk's fresh ETA censuses served from a held capture.
static CENSUSES: AtomicU64 = AtomicU64::new(0);
/// The held trees, between the naval combat part's captures.
static STORE: Mutex<Option<Box<Store>>> = Mutex::new(None);
/// This family's commander-site patches are enabled.
static PATCHED: AtomicBool = AtomicBool::new(false);
/// Each country site's forwarding family has its patch enabled.
static FORWARDERS: [AtomicBool; COUNTRY_SITES] = [const { AtomicBool::new(false) }; COUNTRY_SITES];

/// Diagnostic oracle: also read every reached source's tree afresh and record
/// it in NavalModifierTreeSamples.
pub const ORACLE: bool = cfg!(feature = "naval_tree_oracle");

/// Notifications running now.
pub fn active() -> usize {
    WATCHES
        .active()
        .saturating_add(crate::occupation_changes::active())
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery: nothing observed before it certifies
/// anything.
pub fn boundary(invalidate: bool) {
    WATCHES.boundary(invalidate);
    crate::occupation_changes::boundary(invalidate);
}

/// Off the game thread: size the watches for the receivers the latest
/// capture reached. Nothing is offered before a capture reached any: the
/// first table installs at once, a later larger one at an invalidating hour.
pub fn presize() {
    crate::occupation_changes::prepare();
    let receivers = demand();
    if receivers != 0 {
        WATCHES.offer(receivers);
    }
}

/// Receivers the watches are sized for: the naval combat trees', the naval
/// missions', the ship models' task forces', the raid intel countries' and
/// the reserve trees' countries and commanders.
fn demand() -> usize {
    RECEIVERS
        .load(Ordering::Relaxed)
        .saturating_add(MISSION_RECEIVERS.load(Ordering::Relaxed))
        .saturating_add(SHIP_MODEL_RECEIVERS.load(Ordering::Relaxed))
        .saturating_add(RAID_INTEL_RECEIVERS.load(Ordering::Relaxed))
        .saturating_add(RESERVE_RECEIVERS.load(Ordering::Relaxed))
        .saturating_add(DIVISION_RECEIVERS.load(Ordering::Relaxed))
}

/// A ReserveModifierTrees capture that keyed `receivers` countries and
/// commanders.
pub fn division_trees(receivers: usize) {
    DIVISION_RECEIVERS.fetch_max(receivers, Ordering::Relaxed);
}
pub fn reserve_trees(receivers: usize) {
    if receivers != 0 {
        RESERVE_RECEIVERS.store(receivers, Ordering::Relaxed);
    }
}

/// A RaidIntelInputs walk that keyed `receivers` countries.
pub fn raid_intel(receivers: usize) {
    if receivers != 0 {
        RAID_INTEL_RECEIVERS.store(receivers, Ordering::Relaxed);
    }
}

/// A complete ShipModels capture over `task_forces` task forces, whose
/// terms key each one's country and fleet leader.
pub fn ship_models(task_forces: usize) {
    SHIP_MODEL_RECEIVERS.fetch_max(task_forces.saturating_mul(2), Ordering::Relaxed);
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    WATCHES.collect();
}

/// What a capture sees of the notifications.
pub trait Changes {
    /// `(epoch, sequence)` of one receiver, registering it; `None` when it
    /// cannot be observed.
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)>;
}

pub struct Observer;
impl Changes for Observer {
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        sequence(receiver)
    }
}

/// `(epoch, sequence)` of a CCountry or CUnitLeader, registering it; `None`
/// when it cannot be observed.
pub fn sequence(receiver: usize) -> Option<(u64, u64)> {
    WATCHES.sequence(receiver)
}

/// Certify an already registered receiver; cold admission stays ordered.
pub fn sequence_at(receiver: usize, slot: usize) -> Option<((u64, u64), usize)> {
    WATCHES.observed_sequence_at(receiver, slot)
}

/// [`sequence_at`] that registers a receiver no slot names yet, for range
/// reads that read a cold receiver themselves. Registration is lock-free.
pub fn register_at(receiver: usize, slot: usize) -> Option<((u64, u64), usize)> {
    WATCHES.sequence_at(receiver, slot)
}

/// Provisional range lookup; the ordered consumer still registers the receiver.
pub fn observed_sequence(receiver: usize) -> Option<(u64, u64)> {
    WATCHES.observed_sequence(receiver)
}

/// One counter walk's certified censuses (a roster country's or fleet
/// leader's modifier tree in the fresh ETA graph): `served` from a held
/// capture.
pub fn censuses(served: u64) {
    CENSUSES.fetch_add(served, Ordering::Relaxed);
}

/// One naval mission read: `receivers` keyed.
pub fn missions(receivers: usize) {
    if receivers != 0 {
        MISSION_RECEIVERS.store(receivers, Ordering::Relaxed);
    }
}

/// One carried tree: the certificate taken before its read, the accumulator
/// it was read from and its preorder with display bytes in its own arena.
pub struct Held {
    pub certificate: (u64, u64),
    pub accumulator: usize,
    pub nodes: Vec<RawNode>,
    pub text: Vec<u8>,
}

/// Held trees by receiver (CCountry or CUnitLeader address).
#[derive(Default)]
pub struct Store {
    pub held: HashMap<usize, Held>,
}

impl Held {
    /// The held tree stands for this hour's: the receiver's certificate,
    /// taken before the read, is the held one (no hooked writer touched it)
    /// and it is read from the same accumulator.
    pub fn certified(&self, certificate: Option<(u64, u64)>, accumulator: usize) -> bool {
        certificate == Some(self.certificate) && self.accumulator == accumulator
    }
}

/// Worker: the held trees, or an empty store when none is held or another
/// capture holds them (which never happens: one naval part runs at a time).
pub fn take_store() -> Box<Store> {
    match STORE.try_lock() {
        Ok(mut slot) => slot.take().unwrap_or_default(),
        Err(_) => Box::default(),
    }
}

/// Worker: hold the trees a complete capture read or carried.
pub fn return_store(store: Box<Store>) {
    if let Ok(mut slot) = STORE.try_lock() {
        *slot = Some(store);
    }
}

/// One capture's counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    /// Trees of the kinds that are never carried, read afresh.
    pub uncarried: u64,
    /// Country and commander receivers reached, carried or read.
    pub receivers: u64,
}

/// A worker publishes one capture's counts.
pub fn record(counts: &Counts) {
    for (target, value) in READS
        .iter()
        .zip([counts.reused, counts.read, counts.uncarried])
    {
        target.fetch_add(value, Ordering::Relaxed);
    }
    if counts.receivers != 0 {
        RECEIVERS.store(counts.receivers as usize, Ordering::Relaxed);
    }
}

/// A capture that refused: its store was forgotten.
pub fn failed() {
    READS[3].fetch_add(1, Ordering::Relaxed);
}

pub fn status() -> impl serde::Serialize {
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .zip(&CALLS)
            .map(|(site, n)| (site.name, n.load(Ordering::Relaxed))),
    );
    let reads = READS.each_ref().map(|value| value.load(Ordering::Relaxed));
    crate::status::status!({
        "configured": cfg!(feature = "naval_tree_changes"),
        "oracle": ORACLE,
        "poisoned": WATCHES.poisoned(),
        "reused_trees": reads[0],
        "read_trees": reads[1],
        "uncarried_trees": reads[2],
        "failed_captures": reads[3],
        "missions": crate::status::status!({
            "receivers": MISSION_RECEIVERS.load(Ordering::Relaxed),
            "ship_model_receivers": SHIP_MODEL_RECEIVERS.load(Ordering::Relaxed),
            "raid_intel_receivers": RAID_INTEL_RECEIVERS.load(Ordering::Relaxed),
            "scope": "cumulative since attach; a naval mission country is read within the hour its certificate (its own and its fleet leaders' tree certificates, its navy's stored hubs and its headquarters commanders' skill objects) moves",
        }),
        "counter_censuses": crate::status::status!({
            "served": CENSUSES.load(Ordering::Relaxed),
            "scope": "cumulative since attach; the task-force counter walk's fresh ETA censuses of roster countries' and fleet leaders' modifier trees served from the frame's held capture while the receiver's certificate is still the one taken before the held census was read; a failed Ordinary family holds none",
        }),
        "events": events,
        "watches": WATCHES.status(),
        "scope": "cumulative since attach; a country's or commander's modifier tree a live naval combat reaches is carried while its receiver's certificate holds (no hooked writer touched it since the held read, same accumulator); every other reached tree (uncarried_trees) is read each hour a live combat reaches it"
    })
}

/// Observe while this family's patches and every forwarder's are enabled.
/// Enabling starts a new epoch, so nothing held from before a gap certifies.
fn observe() {
    let enabled =
        PATCHED.load(Ordering::SeqCst) && FORWARDERS.iter().all(|f| f.load(Ordering::SeqCst));
    WATCHES.set_enabled(enabled);
    crate::occupation_changes::forwarder(crate::occupation_changes::Forwarder::Countries, enabled);
}

/// The family patching country site `site` (< [`COUNTRY_SITES`]) enabled or
/// disabled it; outside attach and detach only. A forwarder disables before
/// its patch goes and enables after it is in place.
pub fn forwarder(site: usize, enabled: bool) {
    FORWARDERS[site].store(enabled, Ordering::SeqCst);
    observe();
}

pub struct Hooks {
    hooks: Vec<Hook>,
    occupation: crate::occupation_changes::Hooks,
}
impl Hooks {
    /// The enclosing runtime verified the executable hash before construction.
    /// A differing sentinel leaves the family uninstalled; every tree is then
    /// read afresh each hour.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, String> {
        let mut hooks = Vec::new();
        if cfg!(feature = "naval_tree_changes")
            && MUTATIONS[COUNTRY_SITES..].iter().all(|site| {
                crate::hook::installation::sentinel(memory, base + site.rva, site.sentinel)
            })
        {
            let callbacks: [usize; PATCHED_SITES] = [
                recalculate_leader_modifiers as *const () as usize,
                update_hq_scaled_modifiers as *const () as usize,
                construct_leader as *const () as usize,
                copy_construct_leader as *const () as usize,
                destroy_leader as *const () as usize,
            ];
            for (offset, site) in MUTATIONS[COUNTRY_SITES..].iter().enumerate() {
                let hook = unsafe { Hook::new(base + site.rva, callbacks[offset], site.sentinel) }
                    .map_err(|cause| {
                        format!(
                            "naval_tree_changes hook {} construction failed: {cause}",
                            site.name
                        )
                    })?;
                ORIGINAL[COUNTRY_SITES + offset].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self {
            hooks,
            occupation: unsafe { crate::occupation_changes::Hooks::new(memory, base) }?,
        })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        if !self.occupation.set_enabled(enabled)? {
            return Ok(false);
        }
        if !enabled {
            PATCHED.store(false, Ordering::SeqCst);
            observe();
            crate::retreat_ban_changes::forwarder(false);
            crate::unit_leader_changes::forwarder(false);
            crate::unit_leader_trait_changes::forwarder(false);
        }
        // One census and suspension for the whole family, shared with the
        // other families in a runtime round (`hook::shared`): a thread at any
        // of its sites, or in a tap, holds every site back until the next
        // attempt.
        if !Hook::set_family_enabled_shared_named(
            &mut self.hooks,
            enabled,
            "naval_tree_changes",
            &std::array::from_fn::<_, PATCHED_SITES, _>(|i| MUTATIONS[COUNTRY_SITES + i].name),
        )? {
            return Ok(false);
        }
        PATCHED.store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        observe();
        // The retreat ban family observes the commander sites through these
        // patches.
        crate::retreat_ban_changes::forwarder(enabled && !self.hooks.is_empty());
        crate::unit_leader_changes::forwarder(enabled && !self.hooks.is_empty());
        crate::unit_leader_trait_changes::forwarder(enabled && !self.hooks.is_empty());
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks
            .iter()
            .flat_map(Hook::executable_ranges)
            .chain(self.occupation.executable_ranges())
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // SAFETY: like each Hook's relay, the watch tables are dropped only
        // once the hooks were never enabled, or were removed and drained
        // after the last capture; so are the retreat-ban and trait watches
        // they notify.
        unsafe {
            WATCHES.release();
            crate::retreat_ban_changes::release();
            crate::unit_leader_trait_changes::release();
        }
    }
}

/// Hook path, allocation-free: a writer of `site` is about to change the tree
/// of `receiver`; a commander site also changes its retreat bans, a lifetime
/// site builds or frees the leader, and the recalculation tail and the
/// lifetime sites end every change of its traits. Dropping the guards
/// notifies its return.
fn notify(
    receiver: usize,
    site: usize,
) -> (Notification<'static>, [Option<Notification<'static>>; 3]) {
    use crate::unit_leader_changes::Lifetime;
    use crate::unit_leader_trait_changes::Site as Traits;
    CALLS[site].fetch_add(1, Ordering::Relaxed);
    let (lifetime, traits) = match site.checked_sub(COUNTRY_SITES) {
        Some(0) => (None, Some(Traits::RecalculateModifiers)),
        Some(2) => (Some(Lifetime::Construct), Some(Traits::Construct)),
        Some(3) => (Some(Lifetime::CopyConstruct), Some(Traits::CopyConstruct)),
        Some(4) => (Some(Lifetime::Destroy), Some(Traits::Destroy)),
        _ => (None, None),
    };
    (
        WATCHES.notify(receiver),
        [
            (site >= COUNTRY_SITES).then(|| crate::retreat_ban_changes::forwarded(receiver)),
            lifetime.map(|lifetime| crate::unit_leader_changes::forwarded(receiver, lifetime)),
            traits.map(|site| crate::unit_leader_trait_changes::forwarded(receiver, site)),
        ],
    )
}

/// Hook path, allocation-free: the forwarded country writer `site`
/// (< [`COUNTRY_SITES`]) is about to change `country`'s tree. The patching
/// family's detour holds the guard across the original.
pub fn forwarded(site: usize, country: usize) -> (Notification<'static>, Notification<'static>) {
    let occupation = crate::occupation_changes::changed(country);
    // A country site forwards to no commander family.
    (notify(country, site).0, occupation)
}

// Every site takes its receiver in RCX and at most three more integer or
// pointer arguments; the detour forwards RCX..R9 and returns RAX. The
// notification covers the original in flight.
macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            receiver: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::NavalTreeChanges);
            let _notification = notify(receiver, $site);
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(receiver, second, third, fourth) })
        }
    };
}
// Leader modifier recalculation tail (leader), the tail of recalculate modifiers.
callback!(recalculate_leader_modifiers, COUNTRY_SITES);
// Update hq scaled modifiers (leader).
callback!(update_hq_scaled_modifiers, COUNTRY_SITES + 1);
// Leader construction (leader).
callback!(construct_leader, COUNTRY_SITES + 2);
// Leader copy construction (leader, source).
callback!(copy_construct_leader, COUNTRY_SITES + 3);
// Leader destruction (leader).
callback!(destroy_leader, COUNTRY_SITES + 4);

#[cfg(test)]
mod tests {
    /// Ship models size the watches for their task forces' countries and
    /// fleet leaders, so a save without naval combats or missions still has
    /// a table to certify their terms with.
    #[test]
    fn ship_models_size_the_watches_for_their_task_forces() {
        super::ship_models(700);
        assert!(super::demand() >= 1400);
    }

    use super::*;

    #[test]
    fn a_held_tree_is_carried_only_under_its_certificate_and_accumulator() {
        let mut store = Store::default();
        store.held.insert(
            0x1000,
            Held {
                certificate: (3, 8),
                accumulator: 0x15b8,
                nodes: vec![RawNode::default()],
                text: Vec::new(),
            },
        );
        let held = &store.held[&0x1000];
        assert!(held.certified(Some((3, 8)), 0x15b8));
        assert!(!held.certified(Some((3, 10)), 0x15b8));
        assert!(!held.certified(Some((3, 8)), 0x2000));
        // An unobservable receiver certifies nothing.
        assert!(!held.certified(None, 0x15b8));
    }
}
