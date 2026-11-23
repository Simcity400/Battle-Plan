use crate::{
    hook::Hook,
    hook_timing::CallCounters,
    watch_registry::{Notification, WatchRegistry},
};
use collector_observers::unit_history_changes;
use engine_mutations::army_stat_mutations::MUTATIONS;
use engine_native_memory::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const EXPERIENCE_FAILURE_NAMES: [&str; 7] = [
    "division experience census identity unavailable",
    "division experience writer certificate unavailable",
    "division experience address overflow",
    "division experience unreadable",
    "division experience cache capacity exhausted",
    "division experience cache allocation failed",
    "division experience index allocation failed",
];
static EXPERIENCE_FAILURES: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];
const PROGRESSION_FAILURE_NAMES: [&str; 12] = [
    EXPERIENCE_FAILURE_NAMES[0],
    EXPERIENCE_FAILURE_NAMES[1],
    EXPERIENCE_FAILURE_NAMES[2],
    EXPERIENCE_FAILURE_NAMES[3],
    EXPERIENCE_FAILURE_NAMES[4],
    EXPERIENCE_FAILURE_NAMES[5],
    EXPERIENCE_FAILURE_NAMES[6],
    "division previous template pointer unreadable",
    "module address overflow",
    "division template vtable mismatch",
    "division template handle unreadable",
    "non-null division template has null handle",
];
static PROGRESSION_FAILURES: [AtomicU64; 12] = [const { AtomicU64::new(0) }; 12];
pub fn progression_refused(reason: &'static str) {
    if let Some(index) = PROGRESSION_FAILURE_NAMES
        .iter()
        .position(|name| *name == reason)
    {
        PROGRESSION_FAILURES[index].fetch_add(1, Ordering::Relaxed);
    }
}
pub fn experience_refused(reason: &'static str) {
    if let Some(index) = EXPERIENCE_FAILURE_NAMES
        .iter()
        .position(|name| *name == reason)
    {
        EXPERIENCE_FAILURES[index].fetch_add(1, Ordering::Relaxed);
    }
}

const SITES: usize = MUTATIONS.len();
const REFRESH_COUNTRY_NAMES: usize = 2;
static WATCHES: Watches = Watches::new();

const ROSTER_BATCH: usize = 64;
const NAME_GROUP_MEMBER_FIELDS: [engine_native_memory::memory::RecordField; 1] =
    [engine_native_memory::memory::RecordField {
        source_offset: engine_native::carmy_combat_inputs::NAME_GROUP_MEMBER,
        destination_offset: 0,
        len: 8,
    }];
const NAME_GROUP_MEMBER_LAYOUT: engine_native_memory::memory::RecordLayout<'static> =
    engine_native_memory::memory::RecordLayout::new(&NAME_GROUP_MEMBER_FIELDS, 8)
        .expect("valid record layout");

fn roster_members(
    memory: &impl Memory,
    country: usize,
    mut visit: impl FnMut(usize),
) -> Option<()> {
    let roster = engine_native_memory::cpdx_array::Array::read(
        memory,
        country.checked_add(engine_native::ccountry::DIVISIONS)?,
        8,
    )?;
    let mut pointer_bytes = [0_u8; ROSTER_BATCH * 8];
    let mut pointers = [0_usize; ROSTER_BATCH];
    let mut member_bytes = [0_u8; ROSTER_BATCH * 8];
    let mut first = 0_u32;
    while first < roster.count {
        let count = ((roster.count - first) as usize).min(ROSTER_BATCH);
        let span = roster.slice(first, count as u32, 8).ok()?;
        if !engine_native_memory::read_slice(memory, span.data, &mut pointer_bytes[..count * 8]) {
            return None;
        }
        let mut live = 0_usize;
        for chunk in pointer_bytes[..count * 8].chunks_exact(8) {
            let army = u64::from_le_bytes(chunk.try_into().unwrap());
            if army == 0 {
                continue;
            }
            let pointer = usize::try_from(army).ok()?;
            pointer.checked_add(engine_native::carmy_combat_inputs::NAME_GROUP_MEMBER)?;
            pointers[live] = pointer;
            live += 1;
        }
        if live != 0 {
            if memory.copy_records(
                engine_native_memory::memory::RecordSources::Pointers(&pointers[..live]),
                &NAME_GROUP_MEMBER_LAYOUT,
                &mut member_bytes[..live * 8],
            ) < live
            {
                return None;
            }
            for chunk in member_bytes[..live * 8].chunks_exact(8) {
                let member = u64::from_le_bytes(chunk.try_into().unwrap());
                if member != 0 {
                    visit(usize::try_from(member).ok()?);
                }
            }
        }
        first += count as u32;
    }
    Some(())
}
static EXPERIENCE_PATCHED: AtomicBool = AtomicBool::new(false);
static EXPERIENCE_FORWARDED: AtomicBool = AtomicBool::new(false);
fn experience_enabled() {
    WATCHES.experience.set_enabled(
        EXPERIENCE_PATCHED.load(Ordering::Acquire) && EXPERIENCE_FORWARDED.load(Ordering::Acquire),
    );
}
fn experience_patched(enabled: bool) {
    EXPERIENCE_PATCHED.store(enabled, Ordering::Release);
    experience_enabled();
}
pub fn experience_forwarder(enabled: bool) {
    EXPERIENCE_FORWARDED.store(enabled, Ordering::Release);
    experience_enabled();
}
pub fn experience_damage(army: usize) -> (Notification<'static>, Notification<'static>) {
    (
        WATCHES.experience.notify(army),
        unit_history_changes::notify(army),
    )
}

static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static READS: [AtomicU64; 6] = [const { AtomicU64::new(0) }; 6];
static DIVISIONS: AtomicUsize = AtomicUsize::new(0);
#[cfg(test)]
pub(crate) static HISTORY_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[cfg(test)]
pub(crate) static MEMBERSHIP_TEST: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[cfg(test)]
pub(crate) fn serial(lock: &'static std::sync::Mutex<()>) -> std::sync::MutexGuard<'static, ()> {
    lock.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub struct Watches {
    registry: WatchRegistry,
    experience: WatchRegistry,
    progression: WatchRegistry,
    calls: CallCounters<SITES>,
}

impl Default for Watches {
    fn default() -> Self {
        Self::new()
    }
}

impl Watches {
    pub const fn new() -> Self {
        Self {
            registry: WatchRegistry::marks_only(),
            experience: WatchRegistry::marking(),
            progression: WatchRegistry::marking(),
            calls: CallCounters::new(),
        }
    }

    #[inline]
    pub fn notify(
        &self,
        receiver: usize,
        site: usize,
    ) -> (Notification<'_>, Option<Notification<'_>>) {
        self.calls.increment(site);
        let progression = matches!(
            site,
            6 | 7 | engine_mutations::army_stat_mutations::EXPERIENCE_TEMPLATE_SITE
        )
        .then(|| self.progression.notify(receiver));
        let primary = if site == engine_mutations::army_stat_mutations::EXPERIENCE_TEMPLATE_SITE {
            self.experience.notify(receiver)
        } else if site >= engine_mutations::army_stat_mutations::HISTORY_FIRST {
            unit_history_changes::notify(receiver)
        } else if site >= engine_mutations::army_stat_mutations::EXPERIENCE_FIRST {
            self.experience.notify(receiver)
        } else {
            self.registry.notify(receiver)
        };
        (primary, progression)
    }

    pub fn refreshed_country_names(&self, memory: &impl Memory, country: usize) {
        self.calls.increment(REFRESH_COUNTRY_NAMES);
        if roster_members(memory, country, |member| drop(self.registry.notify(member))).is_none() {
            drop(self.registry.notify_every());
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(armies: usize) -> Self {
        let watches = Self::new();
        watches.registry.offer(armies);
        watches.registry.boundary(false);
        watches.registry.set_enabled(true);
        watches.experience.offer(armies);
        watches.experience.boundary(false);
        watches.experience.set_enabled(true);
        watches.progression.offer(armies);
        watches.progression.boundary(false);
        watches.progression.set_enabled(true);
        watches
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        self.registry.boundary(invalidate);
        self.experience.boundary(invalidate);
        self.progression.boundary(invalidate);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self) -> bool {
        self.registry.poisoned()
    }
}

pub trait Changes: Sync {
    fn progression_sequence(&self, _receiver: usize) -> Option<(u64, u64)> {
        None
    }
    fn experience_sequence(&self, _receiver: usize) -> Option<(u64, u64)> {
        None
    }
    fn experience_range_sequence(&self, _receiver: usize) -> Option<(u64, u64)> {
        None
    }
    fn progression_range_sequence(&self, _receiver: usize) -> Option<(u64, u64)> {
        None
    }
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64>;
    fn take_experience(&self, _visit: &mut dyn FnMut(usize)) -> Option<u64> {
        None
    }
    fn take_progression(&self, _visit: &mut dyn FnMut(usize)) -> Option<u64> {
        None
    }
    fn register(&self, receiver: usize) -> bool;
}

impl Changes for Watches {
    fn progression_sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        self.progression.sequence(receiver)
    }
    fn experience_sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        self.experience.sequence(receiver)
    }
    fn experience_range_sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        self.experience.range_sequence(receiver)
    }
    fn progression_range_sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        self.progression.range_sequence(receiver)
    }
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        self.registry.take(visit)
    }
    fn take_experience(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        self.experience.take(visit)
    }
    fn take_progression(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        self.progression.take(visit)
    }
    fn register(&self, receiver: usize) -> bool {
        self.registry.register(receiver).is_some()
    }
}

pub struct Observer;
impl Changes for Observer {
    fn progression_sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        WATCHES.progression.sequence(receiver)
    }
    fn experience_sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        WATCHES.experience.sequence(receiver)
    }
    fn experience_range_sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        WATCHES.experience.range_sequence(receiver)
    }
    fn progression_range_sequence(&self, receiver: usize) -> Option<(u64, u64)> {
        WATCHES.progression.range_sequence(receiver)
    }
    fn take(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        WATCHES.take(visit)
    }
    fn take_experience(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        WATCHES.experience.take(visit)
    }
    fn take_progression(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        WATCHES.progression.take(visit)
    }
    fn register(&self, receiver: usize) -> bool {
        WATCHES.register(receiver)
    }
}

pub const WITNESS: bool = cfg!(feature = "attack_witness");

pub fn registries() -> [(&'static str, &'static WatchRegistry); 4] {
    [
        ("army_stats", &WATCHES.registry),
        ("army_experience", &WATCHES.experience),
        ("army_progression", &WATCHES.progression),
        ("unit_history", unit_history_changes::registry()),
    ]
}

pub fn active() -> usize {
    WATCHES.registry.active()
        + WATCHES.experience.active()
        + WATCHES.progression.active()
        + unit_history_changes::active()
        + collector_observers::division_panel_changes::active()
}

pub fn boundary(invalidate: bool) {
    unit_history_changes::boundary(invalidate);
    collector_observers::division_panel_changes::boundary(invalidate);
    WATCHES.registry.boundary(invalidate);
    WATCHES.experience.boundary(invalidate);
    WATCHES.progression.boundary(invalidate);
}

pub fn demand_divisions(divisions: usize) {
    DIVISIONS.fetch_max(divisions, Ordering::Relaxed);
    presize();
}

pub fn presize() {
    let divisions = DIVISIONS.load(Ordering::Relaxed);
    unit_history_changes::demand(divisions);
    unit_history_changes::presize();
    collector_observers::division_panel_changes::demand(
        divisions,
        divisions,
        divisions.saturating_mul(2),
    );
    collector_observers::division_panel_changes::presize();
    if divisions != 0 {
        WATCHES.registry.offer(divisions.saturating_mul(2));
        WATCHES.experience.offer(divisions);
        WATCHES.progression.offer(divisions);
    }
}

pub fn collect() {
    WATCHES.registry.collect();
    WATCHES.experience.collect();
    WATCHES.progression.collect();
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    pub refused: u64,
    pub witnessed: u64,
    pub maxima_reused: u64,
    pub names_reused: u64,
}

pub fn record(counts: &Counts, divisions: Option<usize>) {
    for (target, value) in READS.iter().zip([
        counts.reused,
        counts.read,
        counts.refused,
        counts.witnessed,
        counts.maxima_reused,
        counts.names_reused,
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    if let Some(divisions) = divisions {
        DIVISIONS.store(divisions, Ordering::Relaxed);
    }
}

fn hook_failure(site: Option<usize>, action: &str, detail: &str) -> String {
    let sites = site.map_or_else(
        || {
            MUTATIONS
                .iter()
                .map(|site| site.name)
                .collect::<Vec<_>>()
                .join(", ")
        },
        |index| MUTATIONS[index].name.to_owned(),
    );
    format!("army_stat_changes [{sites}] hook {action} failed: {detail}")
}

fn validate_sites(base: usize) -> Result<(), String> {
    for (index, site) in MUTATIONS.iter().enumerate() {
        if let Err(cause) =
            engine_sites::hook(base, site.rva).and_then(|code| code.sentinel::<16>().map(drop))
        {
            return Err(hook_failure(Some(index), "site location", cause));
        }
    }
    Ok(())
}

pub struct Hooks {
    hooks: Vec<Hook>,
    operands: crate::panel_operand_hooks::Hooks,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    /// A withheld site refuses installation with the owning family and site.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, String> {
        let mut hooks = Vec::new();
        if cfg!(feature = "army_stat_changes") {
            validate_sites(base)?;
            let callbacks: [usize; SITES] = [
                update_active_stats as *const () as usize,
                refresh_cached_name as *const () as usize,
                refresh_country_names as *const () as usize,
                award_experience as *const () as usize,
                add_manpower_experience as *const () as usize,
                refresh_experience as *const () as usize,
                rebind_experience as *const () as usize,
                construct_division as *const () as usize,
                construct_division_history as *const () as usize,
                destroy_division_history as *const () as usize,
                append_division_history as *const () as usize,
                apply_division_award as *const () as usize,
                clear_medal_inheritance as *const () as usize,
                construct_medal_store as *const () as usize,
                destroy_medal_store as *const () as usize,
                promote_division_officer as *const () as usize,
                bind_division_template as *const () as usize,
                add_officer_experience as *const () as usize,
                set_officer_experience as *const () as usize,
            ];
            for (index, site) in MUTATIONS.iter().enumerate() {
                let code = engine_sites::hook(base, site.rva)?;
                let hook =
                    unsafe { Hook::new(code.address, callbacks[index], code.sentinel::<16>()?) }
                        .map_err(|reason| hook_failure(Some(index), "construction", &reason))?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self {
            hooks,
            operands: unsafe { crate::panel_operand_hooks::Hooks::new(base) }?,
        })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, String> {
        if !self.operands.set_enabled(enabled)? {
            return Ok(false);
        }
        if !enabled {
            WATCHES.registry.set_enabled(false);
            experience_patched(false);
            WATCHES.progression.set_enabled(false);
            unit_history_changes::patched(false);
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|reason| hook_failure(None, "publication", &reason))?
        {
            return Ok(false);
        }
        WATCHES
            .registry
            .set_enabled(enabled && !self.hooks.is_empty());
        experience_patched(enabled && !self.hooks.is_empty());
        WATCHES
            .progression
            .set_enabled(enabled && !self.hooks.is_empty());
        unit_history_changes::patched(enabled && !self.hooks.is_empty());
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks
            .iter()
            .flat_map(Hook::executable_ranges)
            .chain(self.operands.executable_ranges())
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // SAFETY: like each Hook's relay, the watch tables are dropped only
        // once the hooks were never enabled, or were removed and drained
        // after the last capture.
        unsafe {
            WATCHES.registry.release();
            WATCHES.experience.release();
            WATCHES.progression.release();
            unit_history_changes::release();
        };
    }
}

unsafe extern "system" fn update_active_stats(
    army: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ArmyStatChanges);
    let _notification = WATCHES.notify(army, 0);
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[0].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(army, second, third, fourth) })
}

unsafe extern "system" fn refresh_cached_name(
    member: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ArmyStatChanges);
    let _notification = WATCHES.notify(member, 1);
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[1].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(member, second, third, fourth) })
}

unsafe extern "system" fn refresh_country_names(
    first: usize,
    country: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ArmyStatChanges);
    let running = WATCHES.registry.running();
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[REFRESH_COUNTRY_NAMES].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(first, country, third, fourth) });
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    WATCHES.refreshed_country_names(&memory, country);
    drop(running);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_division_demand_prepares_all_career_receivers_before_first_capture() {
        let _serial = serial(&HISTORY_TEST);
        let _operands = serial(&MEMBERSHIP_TEST);
        unit_history_changes::patched(true);
        unit_history_changes::forwarder(true);
        experience_patched(true);
        experience_forwarder(true);
        WATCHES.progression.set_enabled(true);
        demand_divisions(5_000);
        boundary(true);
        for index in 0..5_000 {
            let army = 0x1000 + index * 0x1000;
            for receiver in [
                army,
                army + engine_native_land::unit_history::OFFICER,
                army + engine_native_land::unit_history::DIVISION_HISTORY,
                0x1000_0000 + index * 0x1000,
            ] {
                assert!(unit_history_changes::sequence(receiver).is_some());
            }
            assert!(WATCHES.experience.sequence(army).is_some());
            assert!(WATCHES.progression.sequence(army).is_some());
        }
        let receiver = 0x1000 + engine_native_land::unit_history::OFFICER;
        let held = unit_history_changes::sequence(receiver).unwrap();
        boundary(false);
        assert_eq!(unit_history_changes::sequence(receiver), Some(held));
        drop(unit_history_changes::notify(receiver));
        assert_ne!(unit_history_changes::sequence(receiver), Some(held));
        unit_history_changes::patched(false);
        unit_history_changes::forwarder(false);
        experience_patched(false);
        experience_forwarder(false);
        WATCHES.progression.set_enabled(false);
    }

    #[test]
    fn hook_errors_name_the_family_sites_and_original_failure() {
        for (index, site) in MUTATIONS.iter().enumerate() {
            let error = hook_failure(Some(index), "construction", "synthetic refusal");
            assert!(error.contains("army_stat_changes"));
            assert!(error.contains(site.name));
            assert!(error.contains("synthetic refusal"));
        }
        let error = hook_failure(None, "publication", "byte verification failed");
        for site in MUTATIONS {
            assert!(error.contains(site.name));
        }
        assert!(error.contains("byte verification failed"));
    }

    #[test]
    fn experience_awards_carry_the_independent_template_conversion_certificate() {
        let watches = Watches::enabled(2);
        let mut held = watches.progression_sequence(0x1000).unwrap();
        let other = watches.progression_sequence(0x2000).unwrap();
        for site in [3, 4, 5, 0] {
            drop(watches.notify(0x1000, site));
            assert_eq!(watches.progression_sequence(0x1000), Some(held));
        }
        for site in [
            6,
            7,
            engine_mutations::army_stat_mutations::EXPERIENCE_TEMPLATE_SITE,
        ] {
            let running = watches.notify(0x1000, site);
            assert!(watches.progression_sequence(0x1000).is_none());
            drop(running);
            let next = watches.progression_sequence(0x1000).unwrap();
            assert_ne!(next, held);
            assert_eq!(watches.progression_sequence(0x2000), Some(other));
            held = next;
        }
    }

    #[test]
    fn experience_writers_do_not_dirty_stat_operands_or_other_divisions() {
        let watches = Watches::enabled(16);
        let first = watches.experience_sequence(0x1000).unwrap();
        let second = watches.experience_sequence(0x2000).unwrap();
        let mut ignored = |_| {};
        let epoch = watches.take(&mut ignored).unwrap();
        assert!(watches.register(0x1000));
        for site in [
            engine_mutations::army_stat_mutations::EXPERIENCE_FIRST,
            engine_mutations::army_stat_mutations::EXPERIENCE_TEMPLATE_SITE,
        ] {
            drop(watches.notify(0x1000, site));
        }
        assert_ne!(watches.experience_sequence(0x1000), Some(first));
        assert_eq!(watches.experience_sequence(0x2000), Some(second));
        let mut touched = Vec::new();
        assert_eq!(
            watches.take(&mut |receiver| touched.push(receiver)),
            Some(epoch)
        );
        assert!(touched.is_empty());
        let held = watches.experience_sequence(0x1000);
        drop(watches.notify(0x1000, 0));
        assert_eq!(watches.experience_sequence(0x1000), held);
        watches.boundary(true);
        assert_ne!(watches.experience_sequence(0x1000), held);
    }

    #[test]
    fn the_writer_marks_only_its_registered_receiver() {
        let watches = Watches::enabled(16);
        let take = |watches: &Watches| {
            let mut touched = Vec::new();
            let epoch = Changes::take(watches, &mut |receiver| touched.push(receiver));
            (epoch, touched)
        };
        let (epoch, _) = take(&watches);
        assert!([0x1000, 0x2000].map(|army| Changes::register(&watches, army)) == [true; 2]);
        drop(watches.notify(0x2000, 0));
        drop(watches.notify(0x3000, 1));
        assert_eq!(take(&watches), (epoch, vec![0x2000]));
        assert_eq!(take(&watches), (epoch, vec![]));
        assert_eq!(watches.calls.total(0), 1);
        let running = watches.notify(0x1000, 0);
        assert_eq!(take(&watches).0, None);
        drop(running);
        assert_eq!(take(&watches), (epoch, vec![0x1000]));
        watches.boundary(true);
        assert_ne!(take(&watches).0, epoch);
    }

    #[test]
    fn the_country_name_refresh_marks_only_its_rosters_name_members() {
        use engine_native::{carmy_combat_inputs::NAME_GROUP_MEMBER, ccountry::DIVISIONS};
        const COUNTRY: usize = 0x100;
        const ROSTER: usize = 0x800;
        let armies = [0x1000_usize, 0, 0x2000];
        let mut bytes = vec![0_u8; 0x3000];
        let mut put = |at: usize, value: &[u8]| bytes[at..at + value.len()].copy_from_slice(value);
        put(COUNTRY + DIVISIONS, &(ROSTER as u64).to_le_bytes());
        put(COUNTRY + DIVISIONS + 8, &3_u32.to_le_bytes());
        put(COUNTRY + DIVISIONS + 12, &3_u32.to_le_bytes());
        for (index, army) in armies.iter().enumerate() {
            put(ROSTER + index * 8, &(*army as u64).to_le_bytes());
        }
        put(0x1000 + NAME_GROUP_MEMBER, &0x9100_u64.to_le_bytes());
        put(0x2000 + NAME_GROUP_MEMBER, &0x9200_u64.to_le_bytes());
        let memory = engine_native_memory::memory::SliceMemory {
            base: 0,
            bytes: &bytes,
        };
        let watches = Watches::enabled(16);
        let take = |watches: &Watches| {
            let mut touched = Vec::new();
            let epoch = Changes::take(watches, &mut |receiver| touched.push(receiver));
            touched.sort_unstable();
            (epoch, touched)
        };
        let (epoch, _) = take(&watches);
        for receiver in [0x1000, 0x2000, 0x9100, 0x9200, 0x3000, 0x9300] {
            assert!(Changes::register(&watches, receiver));
        }
        watches.refreshed_country_names(&memory, COUNTRY);
        assert_eq!(take(&watches), (epoch, vec![0x9100, 0x9200]));
        assert_eq!(watches.calls.total(REFRESH_COUNTRY_NAMES), 1);
        watches.refreshed_country_names(&memory, 0x10_0000);
        let (moved, _) = take(&watches);
        assert!(moved.is_some() && moved != epoch);
    }
}

macro_rules! experience_callback {
    ($name:ident, $index:expr) => {
        unsafe extern "system" fn $name(
            army: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::ArmyStatChanges);
            let _notification = WATCHES.notify(army, $index);
            let _edit = matches!($index, 8 | 9 | 12 | 13 | 14)
                .then(|| unit_history_changes::notify_edit(army));
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$index].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(army, second, third, fourth) })
        }
    };
}
experience_callback!(award_experience, 3);
experience_callback!(add_manpower_experience, 4);
experience_callback!(refresh_experience, 5);
experience_callback!(rebind_experience, 6);
experience_callback!(construct_division, 7);

experience_callback!(construct_division_history, 8);
experience_callback!(destroy_division_history, 9);
experience_callback!(append_division_history, 10);
experience_callback!(clear_medal_inheritance, 12);
experience_callback!(construct_medal_store, 13);
experience_callback!(destroy_medal_store, 14);
experience_callback!(add_officer_experience, 17);
experience_callback!(set_officer_experience, 18);

unsafe extern "system" fn apply_division_award(
    store: usize,
    second: usize,
    third: usize,
    fourth: usize,
    entry: usize,
    pay_cost: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ArmyStatChanges);
    let _notification = WATCHES.notify(store, 11);
    let _edit = unit_history_changes::notify_edit(entry);
    let original: unsafe extern "system" fn(usize, usize, usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[11].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(store, second, third, fourth, entry, pay_cost) })
}

unsafe extern "system" fn promote_division_officer(
    leader: usize,
    history: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::ArmyStatChanges);
    let _notification = WATCHES.notify(history, 15);
    let _edit = unit_history_changes::notify_edit(history);
    let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[15].load(Ordering::Acquire)) };
    timing.original(|| unsafe { original(leader, history, third, fourth) })
}

experience_callback!(
    bind_division_template,
    engine_mutations::army_stat_mutations::EXPERIENCE_TEMPLATE_SITE
);
