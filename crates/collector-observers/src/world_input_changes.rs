use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
pub use engine_mutations::world_input_mutations::Domain;
use engine_mutations::{world_input_mutations::DOMAINS, world_input_mutations::MUTATIONS};
use engine_native_memory::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static POLICY_SCORE_OBSERVED: AtomicBool = AtomicBool::new(false);
static WATCHES: [WatchRegistry; DOMAINS] = [const { WatchRegistry::new() }; DOMAINS];
static CALLS: [AtomicU64; SITES] = [const { AtomicU64::new(0) }; SITES];
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forward {
    DesignBonuses,
    DesignOrganisation,
    DesignMove,
    DesignClone,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ForwardCall {
    pub args: [usize; 4],
    pub before: [Option<u64>; 2],
}
pub type Original = unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize;
pub struct Forwarder {
    pub call: unsafe fn(Forward, ForwardCall, Original) -> usize,
    pub enabled: fn(bool),
}
static BASE: AtomicUsize = AtomicUsize::new(0);
static DESIGN_OBSERVING: AtomicBool = AtomicBool::new(false);
pub fn observe_forwarder(enabled: bool) {
    DESIGN_OBSERVING.store(enabled, Ordering::SeqCst);
}
static DESIGN_FORWARD: AtomicUsize = AtomicUsize::new(0);
static DESIGN_ENABLED: AtomicUsize = AtomicUsize::new(0);
static HOOKS_ENABLED: AtomicBool = AtomicBool::new(false);
static FOG_BUILDINGS_ENABLED: AtomicBool = AtomicBool::new(false);
pub fn forwarder(receiver: Option<Forwarder>) {
    if let Some(receiver) = receiver {
        DESIGN_FORWARD.store(receiver.call as usize, Ordering::Release);
        DESIGN_ENABLED.store(receiver.enabled as usize, Ordering::Release);
        (receiver.enabled)(HOOKS_ENABLED.load(Ordering::Acquire));
    } else {
        forwarder_enabled(false);
        DESIGN_OBSERVING.store(false, Ordering::SeqCst);
        DESIGN_FORWARD.store(0, Ordering::Release);
        DESIGN_ENABLED.store(0, Ordering::Release);
    }
}
fn forwarder_enabled(enabled: bool) {
    let callback = DESIGN_ENABLED.load(Ordering::Acquire);
    if callback != 0 {
        let callback: fn(bool) = unsafe { std::mem::transmute(callback) };
        callback(enabled);
    }
}
fn forwarded_site(site: usize) -> Option<Forward> {
    match site {
        17 => Some(Forward::DesignBonuses),
        18 => Some(Forward::DesignOrganisation),
        19 => Some(Forward::DesignMove),
        20 => Some(Forward::DesignClone),
        _ => None,
    }
}
/// # Safety
/// The arguments are ones `SITE`'s published original accepts with its ABI, and every object they
/// reach stays valid for the reads its site makes; one the original destroys is an identity only
/// once it returns.
#[inline]
unsafe fn invoke<const SITE: usize>(args: [usize; 4]) -> usize {
    let original: Original = unsafe { std::mem::transmute(ORIGINAL[SITE].load(Ordering::Acquire)) };
    let Some(site) = forwarded_site(SITE) else {
        return unsafe { original(args[0], args[1], args[2], args[3]) };
    };
    let receiver = if DESIGN_OBSERVING.load(Ordering::SeqCst) {
        DESIGN_FORWARD.load(Ordering::Acquire)
    } else {
        0
    };
    let mut call = ForwardCall {
        args,
        before: [None, None],
    };
    if receiver != 0 && matches!(site, Forward::DesignMove | Forward::DesignClone) {
        let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
        let base = BASE.load(Ordering::Acquire);
        call.before[0] = engine_native_parts::equipment_variant_identity::identity(&memory, base, args[0]).ok();
        if site == Forward::DesignMove {
            call.before[1] =
                engine_native_parts::equipment_variant_identity::identity(&memory, base, args[1]).ok();
        }
    }
    unsafe { invoke_forwarder(receiver, Some(site), call, original) }
}

/// # Safety
/// `receiver` is zero or a live function with the forwarder's signature and ABI, which accepts
/// `site`, `call` and `original`; `original` is live and accepts the call's four arguments.
unsafe fn invoke_forwarder(
    receiver: usize,
    site: Option<Forward>,
    call: ForwardCall,
    original: Original,
) -> usize {
    let args = call.args;
    if let Some(site) = site.filter(|_| receiver != 0) {
        let receiver: unsafe fn(Forward, ForwardCall, Original) -> usize =
            unsafe { std::mem::transmute(receiver) };
        unsafe { receiver(site, call, original) }
    } else {
        unsafe { original(args[0], args[1], args[2], args[3]) }
    }
}
static NAVY_RESET_FORWARD: AtomicUsize = AtomicUsize::new(0);
static NAVY_RESET_ENABLED_CALLBACK: AtomicUsize = AtomicUsize::new(0);
static NAVY_RESET_ENABLED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[derive(Clone, Copy)]
pub struct NavyResetForwarder {
    pub invoke: unsafe fn(usize, usize, usize, usize, usize) -> usize,
    pub enabled: fn(bool),
}

pub fn navy_reset_forwarder(forwarder: Option<NavyResetForwarder>) {
    let previous = NAVY_RESET_ENABLED_CALLBACK.load(Ordering::Acquire);
    if previous != 0 {
        let callback: fn(bool) = unsafe { std::mem::transmute(previous) };
        callback(false);
    }
    NAVY_RESET_FORWARD.store(
        forwarder.map_or(0, |f| f.invoke as usize),
        Ordering::Release,
    );
    NAVY_RESET_ENABLED_CALLBACK.store(
        forwarder.map_or(0, |f| f.enabled as usize),
        Ordering::Release,
    );
    if let Some(forwarder) = forwarder {
        (forwarder.enabled)(NAVY_RESET_ENABLED.load(Ordering::Acquire));
    }
}
fn navy_reset_enabled(enabled: bool) {
    NAVY_RESET_ENABLED.store(enabled, Ordering::Release);
    let callback = NAVY_RESET_ENABLED_CALLBACK.load(Ordering::Acquire);
    if callback != 0 {
        let callback: fn(bool) = unsafe { std::mem::transmute(callback) };
        callback(enabled);
    }
}
static READS: [[AtomicU64; 2]; DOMAINS] = [const { [const { AtomicU64::new(0) }; 2] }; DOMAINS];

pub type Token = (u64, u64);

pub trait Changes: Sync {
    fn token(&self, domain: Domain) -> Option<Token>;
    fn country(&self, country: usize) -> Option<[(u64, u64); 2]>;
}

pub struct Observer;
impl Changes for Observer {
    fn token(&self, domain: Domain) -> Option<Token> {
        WATCHES[domain as usize].quiet()
    }
    fn country(&self, country: usize) -> Option<[(u64, u64); 2]> {
        Some([
            WATCHES[Domain::DynamicModifiers as usize].sequence(country)?,
            crate::naval_tree_changes::sequence(country)?,
        ])
    }
}

pub fn record_countries(carried: u64, read: u64) {
    let reads = &READS[Domain::DynamicModifiers as usize];
    for (target, value) in reads.iter().zip([carried, read]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
}

pub fn record_daily(domain: Domain, carried: bool, read: bool) {
    for (target, value) in READS[domain as usize].iter().zip([carried, read]) {
        target.fetch_add(u64::from(value), Ordering::Relaxed);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    token: Option<Token>,
    pub carry: bool,
    steady: bool,
}

impl Default for Plan {
    fn default() -> Self {
        Self::READ
    }
}

impl Plan {
    pub const READ: Self = Self {
        token: None,
        carry: false,
        steady: false,
    };

    pub fn read(self, domain: Domain, changes: &(impl Changes + ?Sized)) -> Self {
        Self {
            steady: self.token.is_some() && changes.token(domain) == self.token,
            ..self
        }
    }
}

pub struct Held<P> {
    token: Option<Token>,
    pub payload: P,
}

impl<P> Held<P> {
    pub const fn new(payload: P) -> Self {
        Self {
            token: None,
            payload,
        }
    }

    pub fn plan(
        &self,
        domain: Domain,
        changes: &(impl Changes + ?Sized),
        force_full: bool,
    ) -> Plan {
        let token = changes.token(domain);
        let quiet = !force_full && token.is_some() && self.token == token;
        Plan {
            token,
            carry: quiet,
            steady: false,
        }
    }

    pub fn holds(&self, plan: Plan) -> bool {
        plan.carry && self.token == plan.token
    }

    pub fn carried(&self, domain: Domain) -> &P {
        READS[domain as usize][0].fetch_add(1, Ordering::Relaxed);
        &self.payload
    }

    pub fn settle(
        &mut self,
        domain: Domain,
        plan: Plan,
        read: bool,
        hold: impl FnOnce(&mut P) -> bool,
    ) {
        READS[domain as usize][1].fetch_add(1, Ordering::Relaxed);
        self.token = None;
        if read && plan.steady && hold(&mut self.payload) {
            self.token = plan.token;
        }
    }
}

pub fn assign<T: Copy>(held: &mut Vec<T>, fresh: &[T]) -> bool {
    held.clear();
    if engine_allocation::reserve(held, fresh.len()).is_err() {
        return false;
    }
    held.extend_from_slice(fresh);
    true
}

pub fn active() -> usize {
    WATCHES.iter().map(WatchRegistry::active).sum::<usize>() + crate::doctrine_changes::active()
}

pub fn design_forwarder_active() -> usize {
    WATCHES[Domain::DesignIndustrialOrganisation as usize].active()
}

pub fn design_quiet() -> Option<Token> {
    WATCHES[Domain::DesignIndustrialOrganisation as usize].quiet()
}

pub fn boundary(invalidate: bool) {
    if invalidate {
        WATCHES[Domain::GameFog as usize].boundary(true);
        crate::doctrine_changes::invalidate();
    }
    for watches in WATCHES
        .iter()
        .take(Domain::IndustrialOrganisationRosters as usize)
    {
        watches.boundary(invalidate);
    }
}

pub fn fog_forwarder(enabled: bool) {
    FOG_BUILDINGS_ENABLED.store(enabled, Ordering::SeqCst);
    WATCHES[Domain::GameFog as usize].set_enabled(enabled && HOOKS_ENABLED.load(Ordering::Acquire));
}
pub fn fog_nudge_entered() -> Notification<'static> {
    NUDGED.store(true, Ordering::SeqCst);
    WATCHES[Domain::GameFog as usize].notify(0)
}
static NUDGED: AtomicBool = AtomicBool::new(false);
pub fn map_content_stable() -> bool {
    FOG_BUILDINGS_ENABLED.load(Ordering::SeqCst) && !NUDGED.load(Ordering::SeqCst)
}

pub fn presize(slots: usize) {
    WATCHES[Domain::DynamicModifiers as usize].offer(slots);
}

pub fn prepare_mios(count: usize, invalidate: bool) {
    let watches = &WATCHES[Domain::IndustrialOrganisationRosters as usize];
    watches.offer(count);
    watches.boundary(invalidate);
    WATCHES[Domain::IndustrialOrganisationPolicies as usize].boundary(invalidate);
}
pub fn mio_rosters(pointer: usize) -> Option<Token> {
    WATCHES[Domain::IndustrialOrganisationRosters as usize].sequence(pointer)
}

pub fn mio_policy_reload() -> Notification<'static> {
    WATCHES[Domain::IndustrialOrganisationPolicies as usize].notify(0)
}
pub fn mio_policy_observation(enabled: bool) {
    WATCHES[Domain::IndustrialOrganisationPolicies as usize]
        .set_enabled(enabled && POLICY_SCORE_OBSERVED.load(Ordering::SeqCst));
}
pub fn mio_policies() -> Option<Token> {
    if !crate::country_input_changes::mio_policies_observed()
        || !POLICY_SCORE_OBSERVED.load(Ordering::SeqCst)
    {
        return None;
    }
    WATCHES[Domain::IndustrialOrganisationPolicies as usize].quiet()
}

pub fn prepare_design_mios(count: usize, invalidate: bool) {
    let watches = &WATCHES[Domain::DesignIndustrialOrganisation as usize];
    watches.offer(count);
    watches.boundary(invalidate);
}
pub fn design_mio(pointer: usize) -> Option<Token> {
    WATCHES[Domain::DesignIndustrialOrganisation as usize].sequence(pointer)
}

pub fn collect() {
    for watches in &WATCHES {
        watches.collect();
    }
}

const HOOK_ERRORS: [&str; 27] = [
    "world-input rename_province hook construction failed",
    "world-input reset_province_name hook construction failed",
    "world-input set_region_access_modes hook construction failed",
    "world-input reset_navy hook construction failed",
    "world-input update_threat_total hook construction failed",
    "world-input add_dynamic_modifier hook construction failed",
    "world-input renew_dynamic_modifier hook construction failed",
    "world-input remove_dynamic_modifier hook construction failed",
    "world-input dynamic_modifier_daily_update hook construction failed",
    "world-input mio_queue_append hook construction failed",
    "world-input mio_state_copy hook construction failed",
    "world-input mio_allowed_policies hook construction failed",
    "world-input mio_queue_move hook construction failed",
    "world-input mio_read_state hook construction failed",
    "world-input mio_queue_remove hook construction failed",
    "world-input mio_queue_replace hook construction failed",
    "world-input mio_unlock_trait hook construction failed",
    "world-input design_mio_bonuses hook construction failed",
    "world-input design_mio_link hook construction failed",
    "world-input design_mio_move hook construction failed",
    "world-input design_mio_clone hook construction failed",
    "world-input design_mio_read hook construction failed",
    "world-input mio_policy_score_refresh hook construction failed",
    "world-input fog_enter_game hook construction failed",
    "world-input fog_enter_frontend hook construction failed",
    "world-input fog_leave_nudge hook construction failed",
    "world-input fog_toggle hook construction failed",
];
fn gain_tap_error(index: usize, enabled: bool, cause: String) -> String {
    format!(
        "WorldInputs {} {}: {cause}",
        crate::doctrine_changes::GAIN_SITE_NAMES[index],
        if enabled { "enable" } else { "disable" },
    )
}

pub struct Hooks {
    hooks: Vec<Hook>,
    gain_taps: Vec<crate::hook::Tap>,
    doctrines: bool,
    world_hooks: usize,
}
impl Hooks {
    fn world_inputs_enabled(&self, enabled: bool) -> bool {
        enabled && self.world_hooks == SITES
    }
    /// The runtime's build gate admitted this family before construction.
    /// A differing sentinel leaves the family uninstalled. Existing polled
    /// families read afresh; doctrine tables refuse without their writer coverage.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        BASE.store(base, Ordering::Release);
        let mut hooks = Vec::new();
        let mut doctrines = false;
        let mut gain_taps = Vec::new();
        let mut world_hooks = 0;
        if cfg!(feature = "world_input_changes")
            && MUTATIONS
                .iter()
                .all(|(site, _)| crate::located(base, site.rva).is_ok())
        {
            let callbacks: [usize; SITES] = [
                rename_province as *const () as usize,
                reset_province_name as *const () as usize,
                set_region_access_modes as *const () as usize,
                reset_navy as *const () as usize,
                update_threat_total as *const () as usize,
                add_dynamic_modifier as *const () as usize,
                renew_dynamic_modifier as *const () as usize,
                remove_dynamic_modifier as *const () as usize,
                dynamic_modifier_daily_update as *const () as usize,
                mio_queue_append as *const () as usize,
                mio_state_copy as *const () as usize,
                mio_allowed_policies as *const () as usize,
                mio_queue_move as *const () as usize,
                mio_read_state as *const () as usize,
                mio_queue_remove as *const () as usize,
                mio_queue_replace as *const () as usize,
                mio_unlock_trait as *const () as usize,
                design_mio_bonuses as *const () as usize,
                design_mio_link as *const () as usize,
                design_mio_move as *const () as usize,
                design_mio_clone as *const () as usize,
                design_mio_read as *const () as usize,
                mio_policy_score_refresh as *const () as usize,
                fog_enter_game as *const () as usize,
                fog_enter_frontend as *const () as usize,
                fog_leave_nudge as *const () as usize,
                fog_toggle as *const () as usize,
            ];
            for (index, (site, _)) in MUTATIONS.iter().enumerate() {
                let code = engine_sites::hook(base, site.rva)?;
                let hook =
                    unsafe { Hook::new(code.address, callbacks[index], code.sentinel::<16>()?) }
                        .map_err(|_| HOOK_ERRORS[index])?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
            world_hooks = hooks.len();
            if let (Ok(extra), Ok(taps)) = (
                unsafe { crate::doctrine_changes::hooks(memory, base) },
                unsafe { crate::doctrine_changes::gain_taps(memory, base) },
            ) {
                hooks.extend(extra);
                gain_taps = taps;
                doctrines = true;
            }
        }
        Ok(Self {
            hooks,
            gain_taps,
            doctrines,
            world_hooks,
        })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        if !enabled {
            crate::doctrine_changes::enabled(false);
            HOOKS_ENABLED.store(false, Ordering::Release);
            forwarder_enabled(false);
        }
        if !enabled {
            POLICY_SCORE_OBSERVED.store(false, Ordering::SeqCst);
        }
        if !enabled {
            navy_reset_enabled(false);
            for watches in &WATCHES {
                watches.set_enabled(false);
            }
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|why| format!("WorldInputs native writer state change: {why}"))?
        {
            return Ok(false);
        }
        let mut complete = true;
        for (index, tap) in self.gain_taps.iter_mut().enumerate() {
            complete &= Hook::set_family_enabled_shared(
                std::slice::from_mut(tap.publication_hook()),
                enabled,
            )
            .map_err(|why| gain_tap_error(index, enabled, why))?;
        }
        if !complete {
            return Ok(false);
        }
        POLICY_SCORE_OBSERVED.store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        HOOKS_ENABLED.store(self.world_inputs_enabled(enabled), Ordering::Release);
        forwarder_enabled(HOOKS_ENABLED.load(Ordering::Acquire));
        for watches in &WATCHES {
            watches.set_enabled(enabled && !self.hooks.is_empty());
        }
        WATCHES[Domain::GameFog as usize].set_enabled(
            HOOKS_ENABLED.load(Ordering::Acquire) && FOG_BUILDINGS_ENABLED.load(Ordering::Acquire),
        );

        crate::doctrine_changes::enabled(enabled && self.doctrines);
        navy_reset_enabled(enabled && !self.hooks.is_empty());
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges).chain(
            self.gain_taps
                .iter()
                .flat_map(crate::hook::Tap::executable_ranges),
        )
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        HOOKS_ENABLED.store(false, Ordering::Release);
        forwarder_enabled(false);
        POLICY_SCORE_OBSERVED.store(false, Ordering::SeqCst);
        navy_reset_enabled(false);
        // SAFETY: like each Hook's relay, the domains hold no table; release
        // only starts an epoch once the hooks were removed and drained.
        for watches in &WATCHES {
            unsafe { watches.release() };
        }
        unsafe { crate::doctrine_changes::release() };
    }
}

#[inline]
fn notify<const SITE: usize>(first: usize) -> Notification<'static> {
    CALLS[SITE].fetch_add(1, Ordering::Relaxed);
    let domain = MUTATIONS[SITE].1;
    let receiver = match domain {
        Domain::IndustrialOrganisationRosters | Domain::DesignIndustrialOrganisation => first,
        Domain::DynamicModifiers => unsafe {
            std::ptr::read_volatile(
                first.wrapping_add(engine_mutations::world_input_mutations::CONTAINER_COUNTRY)
                    as *const usize,
            )
        },
        _ => 0,
    };
    WATCHES[domain as usize].notify(receiver)
}

macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system-unwind" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::WorldInputChanges);
            let _notification = notify::<$site>(first);
            timing.original(|| unsafe { invoke::<$site>([first, second, third, fourth]) })
        }
    };
}
callback!(rename_province, 0);
callback!(reset_province_name, 1);
callback!(set_region_access_modes, 2);
unsafe extern "system" fn reset_navy(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::WorldInputChanges);
    let _notification = notify::<3>(first);
    let original = ORIGINAL[3].load(Ordering::Acquire);
    let forward = NAVY_RESET_FORWARD.load(Ordering::Acquire);
    timing.original(|| unsafe {
        dispatch_navy_reset(first, second, third, fourth, original, forward)
    })
}
/// # Safety
/// `forward` is zero or a live function with the navy-reset forwarder's signature and ABI, and
/// `original` is a live function with the hooked call's signature; each accepts the arguments
/// given here.
unsafe fn dispatch_navy_reset(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
    original: usize,
    forward: usize,
) -> usize {
    if forward != 0 {
        let forward: unsafe fn(usize, usize, usize, usize, usize) -> usize =
            unsafe { std::mem::transmute(forward) };
        unsafe { forward(first, second, third, fourth, original) }
    } else {
        let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
            unsafe { std::mem::transmute(original) };
        unsafe { original(first, second, third, fourth) }
    }
}
callback!(update_threat_total, 4);
callback!(add_dynamic_modifier, 5);
callback!(renew_dynamic_modifier, 6);
callback!(remove_dynamic_modifier, 7);
callback!(dynamic_modifier_daily_update, 8);

callback!(mio_queue_append, 9);
callback!(mio_state_copy, 10);
callback!(mio_allowed_policies, 11);
callback!(mio_queue_move, 12);
callback!(mio_read_state, 13);
callback!(mio_queue_remove, 14);
callback!(mio_queue_replace, 15);
callback!(mio_unlock_trait, 16);

callback!(design_mio_bonuses, 17);
callback!(design_mio_link, 18);
callback!(design_mio_read, 21);
unsafe extern "system-unwind" fn design_mio_move(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::WorldInputChanges);
    let _destination = notify::<19>(first);
    let _source = WATCHES[Domain::DesignIndustrialOrganisation as usize].notify(second);
    timing.original(|| unsafe { invoke::<19>([first, second, third, fourth]) })
}
unsafe extern "system-unwind" fn design_mio_clone(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::WorldInputChanges);
    let _destination = notify::<20>(second);
    timing.original(|| unsafe { invoke::<20>([first, second, third, fourth]) })
}

callback!(mio_policy_score_refresh, 22);

callback!(fog_enter_frontend, 24);
callback!(fog_leave_nudge, 25);
callback!(fog_toggle, 26);
unsafe extern "system-unwind" fn fog_enter_game(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::WorldInputChanges);
    let _notification = notify::<23>(a);
    let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[23].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(a, b, c, d, e) })
}

#[cfg(any(test, feature = "test-support"))]
pub fn design_bonus_refresh_for_test(original: Original) -> Original {
    ORIGINAL[17].store(original as usize, Ordering::Release);
    design_mio_bonuses
}

#[cfg(any(test, feature = "test-support"))]
pub fn design_mio_read_for_test(original: Original) -> Original {
    ORIGINAL[21].store(original as usize, Ordering::Release);
    design_mio_read
}

#[cfg(any(test, feature = "test-support"))]
pub fn set_design_watch_enabled_for_test(enabled: bool) {
    WATCHES[Domain::DesignIndustrialOrganisation as usize].set_enabled(enabled);
}

#[cfg(any(test, feature = "test-support"))]
pub struct Isolated {
    pub watches: [WatchRegistry; DOMAINS],
}

#[cfg(any(test, feature = "test-support"))]
impl Default for Isolated {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Isolated {
    pub fn new() -> Self {
        let isolated = Self {
            watches: [const { WatchRegistry::new() }; DOMAINS],
        };
        for watches in &isolated.watches {
            watches.offer(64);
            watches.boundary(false);
            watches.set_enabled(true);
        }
        isolated
    }

    pub fn write(&self, domain: Domain) {
        drop(self.watches[domain as usize].notify(0));
    }

    pub fn write_country(&self, country: usize) {
        drop(self.watches[Domain::DynamicModifiers as usize].notify(country));
    }

    pub fn poisoned(&self, domain: Domain) -> bool {
        self.watches[domain as usize].poisoned()
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Changes for Isolated {
    fn token(&self, domain: Domain) -> Option<Token> {
        self.watches[domain as usize].quiet()
    }
    fn country(&self, country: usize) -> Option<[(u64, u64); 2]> {
        Some([
            self.watches[Domain::DynamicModifiers as usize].sequence(country)?,
            (1, 0),
        ])
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn companion_hook_storage_does_not_withdraw_world_input_coverage() {
        let mut family = std::mem::ManuallyDrop::new(super::Hooks {
            hooks: Vec::new(),
            world_hooks: super::SITES,
            gain_taps: Vec::new(),
            doctrines: false,
        });
        assert!(family.world_inputs_enabled(true));
        assert!(!family.world_inputs_enabled(false));
        family.world_hooks -= 1;
        assert!(!family.world_inputs_enabled(true));
    }
    use super::*;

    static MOVE_CENSUS_PAUSE: std::sync::Barrier = std::sync::Barrier::new(2);
    unsafe extern "system-unwind" fn paused_move(a: usize, b: usize, c: usize, d: usize) -> usize {
        MOVE_CENSUS_PAUSE.wait();
        MOVE_CENSUS_PAUSE.wait();
        a ^ b ^ c ^ d
    }
    #[test]
    fn withdrawn_forwarder_keeps_shared_move_in_census_through_original() {
        observe_forwarder(false);
        let previous = ORIGINAL[19].swap(paused_move as *const () as usize, Ordering::SeqCst);
        let writer = std::thread::spawn(|| unsafe { design_mio_move(1, 2, 4, 8) });
        MOVE_CENSUS_PAUSE.wait();
        let in_original = design_forwarder_active();
        MOVE_CENSUS_PAUSE.wait();
        let result = writer.join().unwrap();
        ORIGINAL[19].store(previous, Ordering::SeqCst);
        assert_eq!(
            in_original, 2,
            "both operands remain counted with dispatch off"
        );
        assert_eq!(design_forwarder_active(), 0);
        assert_eq!(result, 15);
    }

    #[test]
    fn design_forwarder_wraps_one_original_call_and_preserves_every_argument_and_return() {
        thread_local! { static ORDER: std::cell::Cell<u32> = const { std::cell::Cell::new(0) }; }
        fn step(value: u32) {
            ORDER.set(ORDER.get() * 10 + value);
        }
        unsafe extern "system-unwind" fn original(a: usize, b: usize, c: usize, d: usize) -> usize {
            assert_eq!([a, b, c, d], [11, 22, 33, 44]);
            step(2);
            987
        }
        unsafe fn receiver(site: Forward, call: ForwardCall, original: Original) -> usize {
            let args = call.args;
            assert_eq!(call.before, [Some(7), Some(8)]);
            assert_eq!(site, Forward::DesignMove);
            step(1);
            let result = unsafe { original(args[0], args[1], args[2], args[3]) };
            step(3);
            result
        }
        assert_eq!(
            collector_test_support::with_allocations_denied(|| unsafe {
                invoke_forwarder(
                    receiver as *const () as usize,
                    Some(Forward::DesignMove),
                    ForwardCall {
                        args: [11, 22, 33, 44],
                        before: [Some(7), Some(8)],
                    },
                    original,
                )
            }),
            987
        );
        assert_eq!(ORDER.get(), 123);
        ORDER.set(0);
        assert_eq!(
            unsafe {
                invoke_forwarder(
                    0,
                    Some(Forward::DesignMove),
                    ForwardCall {
                        args: [11, 22, 33, 44],
                        before: [None, None],
                    },
                    original,
                )
            },
            987
        );
        assert_eq!(ORDER.get(), 2);
    }

    #[test]
    fn non_design_callback_keeps_its_domain_flight_without_entering_design_dispatch() {
        unsafe extern "system-unwind" fn original(a: usize, b: usize, c: usize, d: usize) -> usize {
            assert_eq!([a, b, c, d], [11, 22, 33, 44]);
            assert_eq!(WATCHES[Domain::Tension as usize].active(), 1);
            assert_eq!(WATCHES[Domain::Tension as usize].quiet(), None);
            assert_eq!(design_forwarder_active(), 0);
            987
        }
        unsafe fn unrelated(_: Forward, _: ForwardCall, _: Original) -> usize {
            panic!("a non-design writer entered the design forwarder")
        }
        forwarder(Some(Forwarder {
            call: unrelated,
            enabled: |_| {},
        }));
        observe_forwarder(true);
        let domain = &WATCHES[Domain::Tension as usize];
        domain.set_enabled(true);
        let before = domain.quiet().unwrap();
        let previous = ORIGINAL[4].swap(original as *const () as usize, Ordering::AcqRel);
        assert_eq!(
            collector_test_support::with_allocations_denied(|| unsafe {
                update_threat_total(11, 22, 33, 44)
            }),
            987
        );
        assert_eq!(domain.quiet(), Some((before.0, before.1 + 1)));
        assert_eq!(domain.active(), 0);
        ORIGINAL[4].store(previous, Ordering::Release);
        forwarder(None);
    }

    #[test]
    fn navy_reset_forwarding_brackets_the_original_and_preserves_arguments_and_return() {
        unsafe extern "system" fn original(state: usize, b: usize, c: usize, d: usize) -> usize {
            let state = unsafe { &*(state as *const AtomicUsize) };
            assert_eq!(state.load(Ordering::SeqCst), 7);
            b.wrapping_add(c).wrapping_add(d)
        }
        unsafe fn forward(a: usize, b: usize, c: usize, d: usize, original: usize) -> usize {
            let state = unsafe { &*(a as *const AtomicUsize) };
            state.store(7, Ordering::SeqCst);
            let native: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(original) };
            let result = unsafe { native(a, b, c, d) };
            state.store(11, Ordering::SeqCst);
            result
        }
        let state = AtomicUsize::new(0);
        let pointer = (&state as *const AtomicUsize) as usize;
        assert_eq!(
            unsafe {
                dispatch_navy_reset(
                    pointer,
                    2,
                    3,
                    4,
                    original as *const () as usize,
                    forward as *const () as usize,
                )
            },
            9
        );
        assert_eq!(state.load(Ordering::SeqCst), 11);
        state.store(7, Ordering::SeqCst);
        assert_eq!(
            unsafe { dispatch_navy_reset(pointer, 2, 3, 4, original as *const () as usize, 0) },
            9
        );
        assert_eq!(state.load(Ordering::SeqCst), 7);
    }

    #[test]
    fn a_held_table_is_carried_only_while_its_domain_is_quiet() {
        let changes = Isolated::new();
        let domain = Domain::ProvinceNames;
        let mut held = Held::new(Vec::<u32>::new());
        let read = |held: &mut Held<Vec<u32>>, plan: Plan, rows: &[u32]| {
            held.settle(domain, plan.read(domain, &changes), true, |held| {
                assign(held, rows)
            })
        };
        let plan = held.plan(domain, &changes, false);
        assert!(!plan.carry, "nothing is held yet");
        read(&mut held, plan, &[1, 2]);
        assert!(held.plan(domain, &changes, false).carry);
        assert!(!held.plan(domain, &changes, true).carry);
        assert_eq!(held.carried(domain), &[1, 2]);
        changes.write(domain);
        let plan = held.plan(domain, &changes, false);
        assert!(!plan.carry);
        changes.write(domain);
        read(&mut held, plan, &[3]);
        assert!(!held.plan(domain, &changes, false).carry);
        let plan = held.plan(domain, &changes, false);
        read(&mut held, plan, &[3]);
        assert!(held.plan(domain, &changes, false).carry);
        changes.write(Domain::Tension);
        assert!(held.plan(domain, &changes, false).carry);
        let plan = held.plan(domain, &changes, false);
        held.settle(domain, plan, false, |_| true);
        assert!(!held.plan(domain, &changes, false).carry);
    }
}

#[cfg(test)]
mod mio_certificate_tests {
    use super::*;
    #[test]
    fn mio_receivers_require_quiet_writers_matching_lifetimes_and_unpoisoned_epochs() {
        for domain in [
            Domain::IndustrialOrganisationRosters,
            Domain::DesignIndustrialOrganisation,
        ] {
            let changes = Isolated::new();
            let watches = &changes.watches[domain as usize];
            let held = watches.sequence(0x1230).unwrap();
            assert_eq!(Some(held), watches.sequence(0x1230));
            let writer = watches.notify(0x1230);
            assert_eq!(watches.sequence(0x1230), None);
            drop(writer);
            assert_ne!(watches.sequence(0x1230), Some(held));
            let current = watches.sequence(0x1230).unwrap();
            drop(watches.notify(0x4560));
            assert_eq!(watches.sequence(0x1230), Some(current));
            watches.boundary(true);
            assert_ne!(watches.sequence(0x1230), Some(current));
            watches.poison();
            assert_eq!(watches.sequence(0x1230), None);
            watches.set_enabled(false);
            assert_eq!(watches.sequence(0x4560), None);
        }
    }
}
