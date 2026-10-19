//! Allocation-free notifications from the native writers of the tables read
//! every hour and carried between hours ([`engine_mutations::world_input_mutations`]):
//! ProvinceNames, NavalRegionAccess, the world tension tables (WorldTension,
//! ThreatSources, ThreatCountryAggregates) and DynamicModifiers. A detour only
//! notifies its domain, counts its site and calls the original; it never
//! changes game behaviour. The whole-table domains register no receivers: a
//! capture compares the token it took before its table's latest complete
//! read with this hour's, and equal tokens prove that no observed writer ran
//! in between ([`WatchRegistry::quiet`]). The dynamic modifier domain
//! notifies the CCountry owning the written container, and a country's
//! certificate joins that sequence to the naval tree family's
//! ([`Changes::country`]).
//!
//! Each table's capture holds its latest complete read in a [`Held`] with the
//! token taken before it. An hour whose token equals the held one publishes
//! the held rows without reading the game; any other hour reads afresh and
//! holds that read when the token stood still across it. The hour of the
//! sampled rotation that a domain's index names reads afresh even when quiet
//! and compares the read with the held rows; a difference is a write no hook
//! saw and disables that domain for the attachment. `world_input_verify`, on
//! by default during the family's burn-in, reads and compares every quiet
//! hour. Anything unobserved (disabled, poisoned, an epoch change, a running
//! writer) yields no token, and the capture reads afresh.
use crate::{
    hook::Hook,
    watch_registry::{Notification, WatchRegistry},
};
pub use engine_mutations::world_input_mutations::Domain;
use engine_mutations::{world_input_mutations::DOMAINS, world_input_mutations::MUTATIONS};
use engine_native::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static POLICY_SCORE_OBSERVED: AtomicBool = AtomicBool::new(false);
static WATCHES: [WatchRegistry; DOMAINS] = [const { WatchRegistry::new() }; DOMAINS];
static CALLS: [AtomicU64; SITES] = [const { AtomicU64::new(0) }; SITES];
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
/// Saved-design receivers share these patches with the MIO certificate. The
/// receiver wraps the original call so it observes the same entry/return scope.
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
/// Attach/detach only, while callbacks are quiescent. No reverse dependency on
/// a consuming capture crate is needed.
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
unsafe fn invoke(site: usize, args: [usize; 4]) -> usize {
    let original: Original = unsafe { std::mem::transmute(ORIGINAL[site].load(Ordering::Acquire)) };
    let receiver = if DESIGN_OBSERVING.load(Ordering::SeqCst) {
        DESIGN_FORWARD.load(Ordering::Acquire)
    } else {
        0
    };
    let site = forwarded_site(site);
    let mut call = ForwardCall {
        args,
        before: [None, None],
    };
    if receiver != 0 && matches!(site, Some(Forward::DesignMove | Forward::DesignClone)) {
        let memory = unsafe { engine_native::memory::LiveMemory::new() };
        let base = BASE.load(Ordering::Acquire);
        call.before[0] = engine_native::cequipment_variant::identity(&memory, base, args[0]).ok();
        if site == Some(Forward::DesignMove) {
            call.before[1] =
                engine_native::cequipment_variant::identity(&memory, base, args[1]).ok();
        }
    }
    unsafe { invoke_forwarder(receiver, site, call, original) }
}

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

/// An additional observer of the navy reset already patched by this family.
/// `invoke` brackets its observation around the original function pointer,
/// preserving all four integer arguments and its return value. `enabled`
/// revokes consumer certificates whenever this patch is absent.
#[derive(Clone, Copy)]
pub struct NavyResetForwarder {
    pub invoke: unsafe fn(usize, usize, usize, usize, usize) -> usize,
    pub enabled: fn(bool),
}

/// Register the sole dependent observer during attach, before patch publication;
/// remove it only after the native callbacks have drained during detach.
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
/// Per domain: quiet hours, read hours, compared hours and mismatches.
static READS: [[AtomicU64; 4]; DOMAINS] = [const { [const { AtomicU64::new(0) }; 4] }; DOMAINS];
static SAMPLED: crate::sampled_verify::Sampled = crate::sampled_verify::Sampled::burned_in(&[
    "a quiet hour's held ProvinceNames rows differ from a fresh read",
    "a quiet hour's held NavalRegionAccess rows differ from a fresh read",
    "a quiet hour's held world tension tables differ from a fresh read",
    "a carried country's DynamicModifiers rows differ from a fresh read",
    "a carried MIO roster differs from a fresh read",
    "a carried MIO policy roster differs from a fresh read",
    "a carried design MIO differs from a fresh read",
    "the carried game fog switch differs from its observed value",
]);
const NAMES: [&str; DOMAINS] = [
    "province_names",
    "region_access",
    "tension",
    "dynamic_modifiers",
    "industrial_organisation_rosters",
    "industrial_organisation_policies",
    "design_industrial_organisation",
    "game_fog",
];

/// Read and compare every quiet hour instead of carrying it (burn-in).
pub const VERIFY: bool = cfg!(feature = "world_input_verify");

/// A domain's quiet token: `(epoch, revision)`.
pub type Token = (u64, u64);

/// The domains as a capture consults them; tests substitute their own, so
/// parallel tests never share the hooks' tokens.
pub trait Changes: Sync {
    /// `domain`'s token while observed with no writer running.
    fn token(&self, domain: Domain) -> Option<Token>;
    /// A held table differed from a fresh read: `domain` certifies nothing
    /// for the rest of the attachment.
    fn mismatch(&self, domain: Domain);
    /// A CCountry's certificate of its DynamicModifiers rows, registering it:
    /// the dynamic modifier domain's sequence (add, renew, remove and the
    /// daily update) and the naval tree family's (`modifier recalculation`,
    /// which rewrites every entry's enabled byte and values, and
    /// `country appearance reset`, which clears them). `None` when either cannot
    /// be observed.
    fn country(&self, country: usize) -> Option<[(u64, u64); 2]>;
}

/// The hooks' domains.
pub struct Observer;
impl Changes for Observer {
    fn token(&self, domain: Domain) -> Option<Token> {
        WATCHES[domain as usize].quiet()
    }
    fn mismatch(&self, domain: Domain) {
        WATCHES[domain as usize].poison();
    }
    fn country(&self, country: usize) -> Option<[(u64, u64); 2]> {
        Some([
            WATCHES[Domain::DynamicModifiers as usize].sequence(country)?,
            crate::naval_tree_changes::sequence(country)?,
        ])
    }
}

/// One DynamicModifiers capture's countries: carried, read, compared with
/// their held rows and differing. A difference has already poisoned the
/// domain ([`Changes::mismatch`]).
pub fn record_countries(carried: u64, read: u64, compared: u64, mismatches: u64) {
    let reads = &READS[Domain::DynamicModifiers as usize];
    for (target, value) in reads.iter().zip([carried, read, compared, mismatches]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    SAMPLED.record(compared, mismatches, Domain::DynamicModifiers as usize);
}

/// Receiver counts from the existing daily MIO and variant workers.
/// These domains do not participate in hourly boundary scans.
pub fn record_daily(domain: Domain, carried: bool, read: bool, compared: bool, mismatch: bool) {
    for (target, value) in READS[domain as usize]
        .iter()
        .zip([carried, read, compared, mismatch])
    {
        target.fetch_add(u64::from(value), Ordering::Relaxed);
    }
}

/// What a capture does this hour, decided before any read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The token taken before the hour's reads.
    token: Option<Token>,
    /// Publish the held table without reading.
    pub carry: bool,
    /// The read is the domain's sampled comparison of a quiet hour.
    compare: bool,
    /// The token stood still across the read ([`Plan::read`]).
    steady: bool,
}

impl Default for Plan {
    fn default() -> Self {
        Self::READ
    }
}

impl Plan {
    /// Read afresh and hold nothing: no store, or another capture holds it.
    pub const READ: Self = Self {
        token: None,
        carry: false,
        compare: false,
        steady: false,
    };

    /// Right after the hour's reads: whether no writer ran across them, so
    /// that the read may be held under the token taken before it.
    pub fn read(self, domain: Domain, changes: &(impl Changes + ?Sized)) -> Self {
        Self {
            steady: self.token.is_some() && changes.token(domain) == self.token,
            ..self
        }
    }
}

/// A table held whole between hours: its latest complete read and the token
/// taken before it.
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

    /// Before the hour's reads. A full hour (load, gap, recovery or a
    /// requested refresh) always reads.
    pub fn plan(
        &self,
        domain: Domain,
        changes: &(impl Changes + ?Sized),
        force_full: bool,
    ) -> Plan {
        let token = changes.token(domain);
        let quiet = !force_full && token.is_some() && self.token == token;
        let compare =
            quiet && crate::sampled_verify::current_burned_in(VERIFY).covers(domain as u64);
        Plan {
            token,
            carry: quiet && !compare,
            compare,
            steady: false,
        }
    }

    /// Whether a carrying `plan` still finds the table it planned to carry
    /// (a capture whose reads and publication run on different threads).
    pub fn holds(&self, plan: Plan) -> bool {
        plan.carry && self.token == plan.token
    }

    /// The held table a carrying plan publishes, counting the quiet hour.
    pub fn carried(&self, domain: Domain) -> &P {
        READS[domain as usize][0].fetch_add(1, Ordering::Relaxed);
        &self.payload
    }

    /// After a fresh read, with the plan [`Plan::read`] returned: `read` when
    /// it completed. A comparing plan compares it with the held table
    /// (`differs`); the read is then held (`hold`, off the game thread,
    /// false when its storage could not grow) when the token stood still
    /// across it, and nothing is held otherwise.
    pub fn settle(
        &mut self,
        domain: Domain,
        changes: &(impl Changes + ?Sized),
        plan: Plan,
        read: bool,
        differs: impl FnOnce(&P) -> bool,
        hold: impl FnOnce(&mut P) -> bool,
    ) {
        let compared = plan.compare && read;
        let mismatch = compared && differs(&self.payload);
        record(domain, compared, mismatch);
        if mismatch {
            changes.mismatch(domain);
        }
        self.token = None;
        if read && plan.steady && hold(&mut self.payload) {
            self.token = plan.token;
        }
    }
}

fn record(domain: Domain, compared: bool, mismatch: bool) {
    let reads = &READS[domain as usize];
    reads[1].fetch_add(1, Ordering::Relaxed);
    reads[2].fetch_add(u64::from(compared), Ordering::Relaxed);
    reads[3].fetch_add(u64::from(mismatch), Ordering::Relaxed);
    if compared {
        SAMPLED.record(1, u64::from(mismatch), domain as usize);
    }
}

/// Replace `held` with `fresh`, worker-only; false when storage could not grow.
pub fn assign<T: Copy>(held: &mut Vec<T>, fresh: &[T]) -> bool {
    held.clear();
    if engine_allocation::reserve(held, fresh.len()).is_err() {
        return false;
    }
    held.extend_from_slice(fresh);
    true
}

/// Notifications running now.
pub fn active() -> usize {
    WATCHES.iter().map(WatchRegistry::active).sum::<usize>() + crate::doctrine_changes::active()
}

/// Shared design callbacks in flight, even when forwarding is withdrawn.
/// Entry precedes the forwarding gate; exit follows the original and receiver's
/// terminal work. Control-thread capability publication uses this census rather
/// than adding another callback counter.
pub fn design_forwarder_active() -> usize {
    WATCHES[Domain::DesignIndustrialOrganisation as usize].active()
}

/// Game thread, in the hourly callback before any capture. `invalidate`
/// after a load, gap or recovery.
pub fn boundary(invalidate: bool) {
    if invalidate {
        WATCHES[Domain::GameFog as usize].boundary(true);
        crate::doctrine_changes::invalidate();
    }
    // MIO certificates are prepared only by their existing daily worker.
    for watches in WATCHES
        .iter()
        .take(Domain::IndustrialOrganisationRosters as usize)
    {
        watches.boundary(invalidate);
    }
}

/// The building family owns nudge construction and forwards that fog writer.
pub fn fog_forwarder(enabled: bool) {
    FOG_BUILDINGS_ENABLED.store(enabled, Ordering::SeqCst);
    WATCHES[Domain::GameFog as usize].set_enabled(enabled && HOOKS_ENABLED.load(Ordering::Acquire));
}
pub fn fog_nudge_entered() -> Notification<'static> {
    WATCHES[Domain::GameFog as usize].notify(0)
}

/// Off the game thread: size the dynamic modifier watches for `slots`
/// country slots.
pub fn presize(slots: usize) {
    WATCHES[Domain::DynamicModifiers as usize].offer(slots);
}

/// Reuse the existing boundary and watch-table provisioning; called only by
/// the daily MIO worker from live demand, never from the hourly callback.
pub fn prepare_mios(count: usize, invalidate: bool) {
    let watches = &WATCHES[Domain::IndustrialOrganisationRosters as usize];
    watches.offer(count);
    watches.boundary(invalidate);
    WATCHES[Domain::IndustrialOrganisationPolicies as usize].boundary(invalidate);
}
/// A MIO's rare rosters; reading this certificate does not drain dirty marks.
pub fn mio_rosters(pointer: usize) -> Option<Token> {
    WATCHES[Domain::IndustrialOrganisationRosters as usize].sequence(pointer)
}
pub fn mio_rosters_mismatch() {
    WATCHES[Domain::IndustrialOrganisationRosters as usize].poison();
}

/// Certificates cover policy identity/name/context/score only. Cost and cooldown
/// are always read in the existing daily production-country census. Reload and
/// score refresh are the writers of the carried caches.
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
pub fn mio_policies_mismatch() {
    WATCHES[Domain::IndustrialOrganisationPolicies as usize].poison();
}

/// Runs only in the existing daily variant-detail acquisition worker.
pub fn prepare_design_mios(count: usize, invalidate: bool) {
    let watches = &WATCHES[Domain::DesignIndustrialOrganisation as usize];
    watches.offer(count);
    watches.boundary(invalidate);
}
pub fn design_mio(pointer: usize) -> Option<Token> {
    WATCHES[Domain::DesignIndustrialOrganisation as usize].sequence(pointer)
}
pub fn design_mio_mismatch() {
    WATCHES[Domain::DesignIndustrialOrganisation as usize].poison();
}

/// Off the game thread: free a replaced watch table.
pub fn collect() {
    for watches in &WATCHES {
        watches.collect();
    }
}

pub fn status() -> impl serde::Serialize {
    let events = crate::status::map(
        MUTATIONS
            .iter()
            .zip(&CALLS)
            .map(|((site, _), n)| (site.name, n.load(Ordering::Relaxed))),
    );
    let domains = crate::status::map(NAMES.iter().zip(&READS).zip(&WATCHES).map(
        |((&name, reads), watches)| {
            let reads = reads.each_ref().map(|value| value.load(Ordering::Relaxed));
            (
                name,
                crate::status::status!({
                    "observed": watches.quiet().is_some() || watches.active() != 0,
                    "poisoned": watches.poisoned(),
                    "quiet": reads[0],
                    "read": reads[1],
                    "compared": reads[2],
                    "mismatches": reads[3],
                }),
            )
        },
    ));
    crate::status::status!({
        "configured": cfg!(feature = "world_input_changes"),
        "doctrines": crate::doctrine_changes::status(),
        "verify": VERIFY,
        "sampled_verified": SAMPLED.verified(),
        "sampled_mismatches": SAMPLED.mismatches(),
        "last_mismatch": SAMPLED.last_mismatch(),
        "slice": SAMPLED.slice(),
        "domains": domains,
        "events": events,
        "watches": WATCHES[Domain::DynamicModifiers as usize].status(),
        "scope": "cumulative since attach; a whole-table domain's token stands still while none of its writers runs (province_names: the rename and reset effects; region_access: region access setter and the navy reset; tension: update total, which every source writer ends in); an hour whose token equals the one taken before the table's latest complete read publishes that read (quiet, in hours), any other hour reads afresh (read); the domain's hour of the rotation (its index in rotation_hours) reads a quiet hour afresh and compares it (compared), as verify mode does every quiet hour; dynamic_modifiers counts countries instead: a country whose certificate (the add, renew, remove and daily update sites naming its container, and the naval tree family's modifier recalculation and country appearance reset) is the held read's is carried (quiet), and each country is read afresh and compared on its day of the rotation by id; the first mismatch disables that domain"
    })
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
    /// The enclosing runtime verified the executable hash before construction.
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
            && MUTATIONS.iter().all(|(site, _)| {
                crate::hook::installation::sentinel(memory, base + site.rva, site.sentinel)
            })
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
                let hook = unsafe { Hook::new(base + site.rva, callbacks[index], site.sentinel) }
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
        // One census and suspension for the whole family, shared with the
        // other families in a runtime round (`hook::shared`): a thread at any
        // of its sites, or in a tap, holds every site back until the next
        // attempt.
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

/// Hook path: a writer of `site` ([`MUTATIONS`] index) is about to run;
/// `first` is its first argument.
fn notify(site: usize, first: usize) -> Notification<'static> {
    CALLS[site].fetch_add(1, Ordering::Relaxed);
    let domain = MUTATIONS[site].1;
    let receiver = match domain {
        // The container the writer is about to write names its country (a
        // state's names none). Only its constructor writes the link.
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

// Each site takes at most four integer or pointer arguments (RCX..R9) and
// returns RAX unchanged from its original. The notification covers the
// original in flight.
macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system-unwind" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::WorldInputChanges);
            let _notification = notify($site, first);
            timing.original(|| unsafe { invoke($site, [first, second, third, fourth]) })
        }
    };
}
// Rename province (effect).
callback!(rename_province, 0);
// Reset province name (effect, scope).
callback!(reset_province_name, 1);
// Set region access modes (navy, changes).
callback!(set_region_access_modes, 2);
// Strategic navy reset keeps its sole patch owner and forwards any dependent
// observation around the same original call, without changing this domain.
unsafe extern "system" fn reset_navy(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::WorldInputChanges);
    let _notification = notify(3, first);
    let original = ORIGINAL[3].load(Ordering::Acquire);
    let forward = NAVY_RESET_FORWARD.load(Ordering::Acquire);
    timing.original(|| unsafe {
        dispatch_navy_reset(first, second, third, fourth, original, forward)
    })
}
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
// Update threat total (manager).
callback!(update_threat_total, 4);
// Add dynamic modifier (container, definition, scope, days).
callback!(add_dynamic_modifier, 5);
// Renew dynamic modifier (container, definition, scope, days).
callback!(renew_dynamic_modifier, 6);
// Remove dynamic modifier (container, definition, scope).
callback!(remove_dynamic_modifier, 7);
// Dynamic modifier daily update (container).
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
// Moving consumes the source bonus store as well as replacing the destination.
unsafe extern "system-unwind" fn design_mio_move(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::WorldInputChanges);
    let _destination = notify(19, first);
    let _source = WATCHES[Domain::DesignIndustrialOrganisation as usize].notify(second);
    timing.original(|| unsafe { invoke(19, [first, second, third, fourth]) })
}
// CloneInto receives the destination in its second argument.
unsafe extern "system-unwind" fn design_mio_clone(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::WorldInputChanges);
    let _destination = notify(20, second);
    timing.original(|| unsafe { invoke(20, [first, second, third, fourth]) })
}

callback!(mio_policy_score_refresh, 22);

callback!(fog_enter_frontend, 24);
callback!(fog_leave_nudge, 25);
callback!(fog_toggle, 26);
// This constructor has a fifth integer/pointer argument in its caller's stack.
unsafe extern "system-unwind" fn fog_enter_game(
    a: usize,
    b: usize,
    c: usize,
    d: usize,
    e: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::WorldInputChanges);
    let _notification = notify(23, a);
    let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[23].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(a, b, c, d, e) })
}

/// Tests: domains of one test's own.
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
    /// Observed, as installed hooks would be.
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

    /// A writer of `domain` runs and returns.
    pub fn write(&self, domain: Domain) {
        drop(self.watches[domain as usize].notify(0));
    }

    /// A dynamic modifier writer of `country`'s container runs and returns.
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
    fn mismatch(&self, domain: Domain) {
        self.watches[domain as usize].poison();
    }
    /// The naval tree half is held still.
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
    fn doctrine_gain_publication_error_keeps_site_action_and_cause() {
        for (index, site) in crate::doctrine_changes::GAIN_SITE_NAMES.iter().enumerate() {
            for enabled in [false, true] {
                let cause = "synthetic byte verification failure";
                let error = super::gain_tap_error(index, enabled, cause.into());
                assert!(error.contains("WorldInputs"));
                assert!(error.contains(*site));
                assert!(error.contains(if enabled { "enable" } else { "disable" }));
                assert!(error.contains(cause));
            }
        }
    }

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
    fn callbacks_follow_the_site_order() {
        assert_eq!(
            MUTATIONS[..9]
                .iter()
                .map(|(site, _)| site.name)
                .collect::<Vec<_>>(),
            [
                "rename_province",
                "reset_province_name",
                "set_region_access_modes",
                "reset_navy",
                "update_threat_total",
                "add_dynamic_modifier",
                "renew_dynamic_modifier",
                "remove_dynamic_modifier",
                "dynamic_modifier_daily_update",
            ]
        );
        for (index, forward, name) in [
            (17, Forward::DesignBonuses, "design_mio_bonuses"),
            (18, Forward::DesignOrganisation, "design_mio_link"),
            (19, Forward::DesignMove, "design_mio_move"),
            (20, Forward::DesignClone, "design_mio_clone"),
        ] {
            assert_eq!(forwarded_site(index), Some(forward));
            assert_eq!(MUTATIONS[index].0.name, name);
        }
        assert_eq!(forwarded_site(16), None);
        assert_eq!(forwarded_site(21), None);
    }

    /// A quiet hour carries the held read; a write, a full hour or an
    /// unobserved domain reads afresh, and a read the token did not stand
    /// still across is not held.
    #[test]
    fn a_held_table_is_carried_only_while_its_domain_is_quiet() {
        // Not the domain's comparison hour (its index, 0).
        crate::sampled_verify::begin_hour(5);
        let changes = Isolated::new();
        let domain = Domain::ProvinceNames;
        let mut held = Held::new(Vec::<u32>::new());
        let read = |held: &mut Held<Vec<u32>>, plan: Plan, rows: &[u32]| {
            held.settle(
                domain,
                &changes,
                plan.read(domain, &changes),
                true,
                |held| held != rows,
                |held| assign(held, rows),
            )
        };
        let plan = held.plan(domain, &changes, false);
        assert!(!plan.carry, "nothing is held yet");
        read(&mut held, plan, &[1, 2]);
        assert!(held.plan(domain, &changes, false).carry || VERIFY);
        assert!(!held.plan(domain, &changes, true).carry);
        assert_eq!(held.carried(domain), &[1, 2]);
        changes.write(domain);
        let plan = held.plan(domain, &changes, false);
        assert!(!plan.carry);
        // A writer runs during the read: the read is published, not held.
        changes.write(domain);
        read(&mut held, plan, &[3]);
        assert!(!held.plan(domain, &changes, false).carry);
        let plan = held.plan(domain, &changes, false);
        read(&mut held, plan, &[3]);
        assert!(held.plan(domain, &changes, false).carry || VERIFY);
        // Another domain's writer leaves this one quiet.
        changes.write(Domain::Tension);
        assert!(held.plan(domain, &changes, false).carry || VERIFY);
        // A capture whose read did not complete holds nothing.
        let plan = held.plan(domain, &changes, false);
        held.settle(domain, &changes, plan, false, |_| false, |_| true);
        assert!(!held.plan(domain, &changes, false).carry);
    }

    /// The domain's rotation hour reads a quiet hour afresh and compares it;
    /// a difference poisons the domain, which then never carries.
    #[test]
    fn a_quiet_hour_that_differs_from_the_held_read_poisons_its_domain() {
        let changes = Isolated::new();
        let domain = Domain::RegionAccess;
        let mut held = Held::new(Vec::<u32>::new());
        let plan = held.plan(domain, &changes, false);
        held.settle(
            domain,
            &changes,
            plan.read(domain, &changes),
            true,
            |_| false,
            |held| assign(held, &[7]),
        );
        crate::sampled_verify::begin_hour(domain as i32);
        let plan = held.plan(domain, &changes, false);
        assert!(!plan.carry && plan.compare);
        held.settle(
            domain,
            &changes,
            plan.read(domain, &changes),
            true,
            |held| held != &[8],
            |held| assign(held, &[8]),
        );
        assert!(changes.poisoned(domain));
        assert_eq!(changes.token(domain), None);
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
            // A reused pointer after world invalidation cannot certify old rows.
            watches.boundary(true);
            assert_ne!(watches.sequence(0x1230), Some(current));
            watches.poison();
            assert_eq!(watches.sequence(0x1230), None);
            watches.set_enabled(false);
            assert_eq!(watches.sequence(0x4560), None);
        }
    }
}
