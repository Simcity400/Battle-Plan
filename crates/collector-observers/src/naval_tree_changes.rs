use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
use engine_mutations::{naval_tree_mutations::COUNTRY_SITES, naval_tree_mutations::MUTATIONS};
use engine_native_memory::Memory;
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
const PATCHED_SITES: usize = SITES - COUNTRY_SITES;
static WATCHES: WatchRegistry = WatchRegistry::new();
static CALLS: [AtomicU64; SITES] = [const { AtomicU64::new(0) }; SITES];
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static READS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static RECEIVERS: AtomicUsize = AtomicUsize::new(0);
static MISSION_RECEIVERS: AtomicUsize = AtomicUsize::new(0);
static SHIP_MODEL_RECEIVERS: AtomicUsize = AtomicUsize::new(0);
static DIVISION_RECEIVERS: AtomicUsize = AtomicUsize::new(0);
static RESERVE_RECEIVERS: AtomicUsize = AtomicUsize::new(0);
static RAID_INTEL_RECEIVERS: AtomicUsize = AtomicUsize::new(0);
static OPERATIVE_EFFECT_RECEIVERS: AtomicUsize = AtomicUsize::new(0);
static CENSUSES: AtomicU64 = AtomicU64::new(0);
static STORE: Mutex<Option<Box<Store>>> = Mutex::new(None);
static PATCHED: AtomicBool = AtomicBool::new(false);
static FORWARDERS: [AtomicBool; COUNTRY_SITES] = [const { AtomicBool::new(false) }; COUNTRY_SITES];

pub const ORACLE: bool = cfg!(feature = "naval_tree_oracle");

pub fn active() -> usize {
    WATCHES
        .active()
        .saturating_add(crate::occupation_changes::active())
}

pub fn boundary(invalidate: bool) {
    WATCHES.boundary(invalidate);
    crate::occupation_changes::boundary(invalidate);
}

pub fn presize() {
    crate::occupation_changes::prepare();
    let receivers = demand();
    if receivers != 0 {
        WATCHES.offer(receivers);
    }
}

fn demand() -> usize {
    RECEIVERS
        .load(Ordering::Relaxed)
        .saturating_add(MISSION_RECEIVERS.load(Ordering::Relaxed))
        .saturating_add(SHIP_MODEL_RECEIVERS.load(Ordering::Relaxed))
        .saturating_add(RAID_INTEL_RECEIVERS.load(Ordering::Relaxed))
        .saturating_add(OPERATIVE_EFFECT_RECEIVERS.load(Ordering::Relaxed))
        .saturating_add(RESERVE_RECEIVERS.load(Ordering::Relaxed))
        .saturating_add(DIVISION_RECEIVERS.load(Ordering::Relaxed))
}

pub fn division_trees(receivers: usize) {
    DIVISION_RECEIVERS.fetch_max(receivers, Ordering::Relaxed);
}
pub fn reserve_trees(receivers: usize) {
    if receivers != 0 {
        RESERVE_RECEIVERS.store(receivers, Ordering::Relaxed);
    }
}

pub fn raid_intel(receivers: usize) {
    if receivers != 0 {
        RAID_INTEL_RECEIVERS.store(receivers, Ordering::Relaxed);
    }
}

pub fn operative_effects(receivers: usize) {
    OPERATIVE_EFFECT_RECEIVERS.fetch_max(receivers, Ordering::Relaxed);
}

pub fn ship_models(task_forces: usize) {
    SHIP_MODEL_RECEIVERS.fetch_max(task_forces.saturating_mul(2), Ordering::Relaxed);
}

pub fn collect() {
    WATCHES.collect();
}

pub trait Changes {
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)>;
}

pub struct Observer;
impl Changes for Observer {
    fn sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        sequence(receiver)
    }
}

pub fn sequence(receiver: usize) -> Option<(u64, u64)> {
    WATCHES.sequence(receiver)
}

pub fn sequence_at(receiver: usize, slot: usize) -> Option<((u64, u64), usize)> {
    WATCHES.observed_sequence_at(receiver, slot)
}

pub fn register_at(receiver: usize, slot: usize) -> Option<((u64, u64), usize)> {
    WATCHES.sequence_at(receiver, slot)
}

pub fn observed_sequence(receiver: usize) -> Option<(u64, u64)> {
    WATCHES.observed_sequence(receiver)
}

pub fn censuses(served: u64) {
    CENSUSES.fetch_add(served, Ordering::Relaxed);
}

pub fn missions(receivers: usize) {
    if receivers != 0 {
        MISSION_RECEIVERS.store(receivers, Ordering::Relaxed);
    }
}

pub struct Held {
    pub certificate: (u64, u64),
    pub accumulator: usize,
    pub nodes: Vec<RawNode>,
    pub text: Vec<u8>,
}

#[derive(Default)]
pub struct Store {
    pub held: HashMap<usize, Held>,
}

impl Held {
    pub fn certified(&self, certificate: Option<(u64, u64)>, accumulator: usize) -> bool {
        certificate == Some(self.certificate) && self.accumulator == accumulator
    }
}

pub fn take_store() -> Box<Store> {
    match STORE.try_lock() {
        Ok(mut slot) => slot.take().unwrap_or_default(),
        Err(_) => Box::default(),
    }
}

pub fn return_store(store: Box<Store>) {
    if let Ok(mut slot) = STORE.try_lock() {
        *slot = Some(store);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    pub uncarried: u64,
    pub receivers: u64,
}

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

pub fn failed() {
    READS[3].fetch_add(1, Ordering::Relaxed);
}

fn observe() {
    let enabled =
        PATCHED.load(Ordering::SeqCst) && FORWARDERS.iter().all(|f| f.load(Ordering::SeqCst));
    WATCHES.set_enabled(enabled);
    crate::occupation_changes::forwarder(crate::occupation_changes::Forwarder::Countries, enabled);
}

pub fn forwarder(site: usize, enabled: bool) {
    FORWARDERS[site].store(enabled, Ordering::SeqCst);
    observe();
}

pub struct Hooks {
    hooks: Vec<Hook>,
    occupation: crate::occupation_changes::Hooks,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
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
            && MUTATIONS[COUNTRY_SITES..]
                .iter()
                .all(|site| crate::located(base, site.rva).is_ok())
        {
            let callbacks: [usize; PATCHED_SITES] = [
                recalculate_leader_modifiers as *const () as usize,
                update_hq_scaled_modifiers as *const () as usize,
                construct_leader as *const () as usize,
                copy_construct_leader as *const () as usize,
                destroy_leader as *const () as usize,
            ];
            for (offset, site) in MUTATIONS[COUNTRY_SITES..].iter().enumerate() {
                let code = engine_sites::hook(base, site.rva)?;
                let hook =
                    unsafe { Hook::new(code.address, callbacks[offset], code.sentinel::<16>()?) }
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

pub fn forwarded(site: usize, country: usize) -> (Notification<'static>, Notification<'static>) {
    let occupation = crate::occupation_changes::changed(country);
    (notify(country, site).0, occupation)
}

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
callback!(recalculate_leader_modifiers, COUNTRY_SITES);
callback!(update_hq_scaled_modifiers, COUNTRY_SITES + 1);
callback!(construct_leader, COUNTRY_SITES + 2);
callback!(copy_construct_leader, COUNTRY_SITES + 3);
callback!(destroy_leader, COUNTRY_SITES + 4);

#[cfg(test)]
mod tests {

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
        assert!(!held.certified(None, 0x15b8));
    }
}
