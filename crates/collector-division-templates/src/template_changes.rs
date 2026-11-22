use collector_certificates::watch_registry::{Notification, WatchRegistry};
use collector_hooks::hook::Hook;
use engine_mutations::{
    template_mutations::DATA, template_mutations::MUTATIONS, template_mutations::Receiver,
};
use engine_native::Memory;
use std::{
    mem::ManuallyDrop,
    ops::Range,
    sync::Mutex,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct HqSource {
    source: usize,
    requested: u32,
    template: Option<(u64, u64)>,
    holder: Option<(u64, u64)>,
}
static HQ_SOURCES: Mutex<Vec<(usize, HqSource)>> = Mutex::new(Vec::new());
static HQ_SOURCE_BYTES: AtomicUsize = AtomicUsize::new(0);

pub fn track_hq_holder(holder: usize) -> Result<(), &'static str> {
    if holder != 0 {
        let mut sources = HQ_SOURCES
            .lock()
            .map_err(|_| "HQ source certificate store poisoned")?;
        if let Err(at) = sources.binary_search_by_key(&holder, |&(holder, _)| holder) {
            engine_allocation::reserve(&mut *sources, 1)
                .map_err(|_| "HQ source certificate allocation failed")?;
            sources.insert(at, (holder, HqSource::default()));
            HQ_SOURCE_BYTES.store(
                sources.capacity() * size_of::<(usize, HqSource)>(),
                Ordering::Relaxed,
            );
        }
    }
    Ok(())
}
pub fn forget_hq_holder(holder: usize) {
    if let Ok(mut sources) = HQ_SOURCES.lock()
        && let Ok(at) = sources.binary_search_by_key(&holder, |&(holder, _)| holder)
    {
        sources.remove(at);
    }
}

fn same_hq_source(held: HqSource, current: HqSource) -> bool {
    current.template.is_some() && current.holder.is_some() && held == current
}

const SITES: usize = MUTATIONS.len();
const UNIT_TYPE_SITE: usize = 23;
const AI_ROLE_SITE: usize = 25;
static WATCHES: Watches = Watches::new();
static BASE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static READS: [AtomicU64; 3] = [const { AtomicU64::new(0) }; 3];
static HOUR: [AtomicU64; 2] = [const { AtomicU64::new(0) }; 2];

static TEMPLATES: AtomicUsize = AtomicUsize::new(0);
static COUNTRIES: AtomicUsize = AtomicUsize::new(0);
static HQ_GROUPS: AtomicUsize = AtomicUsize::new(0);

pub struct Watches {
    registry: WatchRegistry,
    calls: [AtomicU64; SITES],
    unchanged_unit_type_writes: AtomicU64,
    unchanged_ai_role_rebuilds: AtomicU64,
}

impl Watches {
    pub const fn new() -> Self {
        Self {
            registry: WatchRegistry::new(),
            calls: [const { AtomicU64::new(0) }; SITES],
            unchanged_unit_type_writes: AtomicU64::new(0),
            unchanged_ai_role_rebuilds: AtomicU64::new(0),
        }
    }

    pub fn notify(&self, template: usize, site: usize) -> Notification<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        self.registry.notify(template)
    }

    fn unit_type_notification(
        &self,
        memory: &impl Memory,
        data: usize,
        value: usize,
    ) -> Option<Notification<'_>> {
        self.calls[UNIT_TYPE_SITE].fetch_add(1, Ordering::Relaxed);
        let template = data.checked_sub(DATA).filter(|&pointer| pointer != 0);
        let unchanged = template.is_some()
            && data
                .checked_add(
                    engine_native::creferenced_division_template::UNIT_TYPE_OVERRIDE - DATA,
                )
                .and_then(|at| engine_native::read::<u32>(memory, at))
                == Some(value as u32);
        if unchanged {
            self.unchanged_unit_type_writes
                .fetch_add(1, Ordering::Relaxed);
            None
        } else {
            Some(match template {
                Some(template) => self.registry.notify(template),
                None => self.registry.notify_every(),
            })
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(objects: usize) -> Self {
        let watches = Self::new();
        watches.registry.offer(objects);
        watches.registry.boundary(false);
        watches.registry.set_enabled(true);
        watches
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self) -> bool {
        self.registry.poisoned()
    }
}

pub trait Changes: Sync {
    fn fresh_only(&self) -> bool {
        false
    }
    fn quiet(&self) -> Option<(u64, u64)>;
    fn sequence(&self, template: usize) -> Option<(u64, u64)>;
    fn deployment_reset_sequence(&self, status: usize) -> Option<(u64, u64)> {
        self.sequence(status)
    }
    fn core_dependencies(&self) -> Option<(u64, u64)> {
        None
    }
}

impl Changes for Watches {
    fn quiet(&self) -> Option<(u64, u64)> {
        self.registry.quiet()
    }
    fn sequence(&self, template: usize) -> Option<(u64, u64)> {
        self.registry.sequence(template)
    }
}

pub struct Observer;
#[cfg(any(test, feature = "test-support"))]
pub fn unsized_observer_for_tests() -> Observer {
    WATCHES.registry.set_enabled(true);
    Observer
}
impl Changes for Observer {
    fn fresh_only(&self) -> bool {
        collector_hook_failures::failed(collector_hook_failures::Family::Templates)
            && collector_hook_failures::fatal_installation_error().is_none()
    }
    fn quiet(&self) -> Option<(u64, u64)> {
        (!collector_hook_failures::failed(collector_hook_failures::Family::Templates))
            .then(|| WATCHES.quiet())
            .flatten()
    }
    fn sequence(&self, template: usize) -> Option<(u64, u64)> {
        (!collector_hook_failures::failed(collector_hook_failures::Family::Templates))
            .then(|| WATCHES.sequence(template))
            .flatten()
    }
    fn deployment_reset_sequence(&self, status: usize) -> Option<(u64, u64)> {
        collector_model_inputs::local_subunit_changes::Changes::sequence(
            &collector_model_inputs::local_subunit_changes::Observer,
            status,
        )
    }
    fn core_dependencies(&self) -> Option<(u64, u64)> {
        collector_model_inputs::local_subunit_changes::Changes::quiet(
            &collector_model_inputs::local_subunit_changes::Observer,
        )
    }
}

pub fn active() -> usize {
    WATCHES.registry.active()
}

pub fn boundary(invalidate: bool) {
    WATCHES.registry.boundary(invalidate);
}

fn offer(templates: usize, countries: usize, hq_groups: usize) {
    let receivers = templates
        .saturating_add(countries.saturating_mul(3))
        .saturating_add(hq_groups.saturating_mul(2));
    if receivers != 0 {
        WATCHES.registry.offer(receivers);
    }
}

pub(crate) fn prepare(templates: usize, countries: usize) {
    TEMPLATES.fetch_max(templates, Ordering::Relaxed);
    record_countries(countries);
    // SAFETY: Crate-internal sizing helper called from `division_templates::Capacities::presize`.
    unsafe { presize() };
}

pub(crate) fn prepare_hq_groups(groups: usize) {
    record_hq_groups(groups);
    // SAFETY: Crate-internal sizing helper called from `group_counters::Capacities::presize`.
    unsafe { presize() };
}

/// # Safety
/// Run only during attach or the active session's serial sizing pass (or an isolated test), while
/// template-change watches remain live and teardown is excluded.
pub unsafe fn presize() {
    offer(
        TEMPLATES.load(Ordering::Relaxed),
        COUNTRIES.load(Ordering::Relaxed),
        HQ_GROUPS.load(Ordering::Relaxed),
    );
}

pub(crate) fn record_countries(countries: usize) {
    COUNTRIES.fetch_max(countries, Ordering::Relaxed);
}
pub(crate) fn record_hq_groups(groups: usize) {
    HQ_GROUPS.fetch_max(groups, Ordering::Relaxed);
}

pub fn collect() {
    WATCHES.registry.collect();
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub carried: u64,
    pub read: u64,
    pub rereads: u64,
}

pub(crate) fn record(counts: &Counts, templates: Option<usize>) {
    for (target, value) in READS
        .iter()
        .zip([counts.carried, counts.read, counts.rereads])
    {
        target.fetch_add(value, Ordering::Relaxed);
    }
    for (target, value) in HOUR.iter().zip([counts.carried, counts.read]) {
        target.store(value, Ordering::Relaxed);
    }
    if let Some(templates) = templates {
        TEMPLATES.store(templates, Ordering::Relaxed);
    }
}

pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// # Safety
    /// `base` identifies the authenticated executable in this process. The caller owns these sites
    /// exclusively and restores the hooks and quiesces native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        BASE.store(base, Ordering::Release);
        let mut located = [(0, [0; 16]); SITES];
        if cfg!(feature = "template_changes")
            && MUTATIONS.iter().zip(&mut located).all(|((site, _), slot)| {
                collector_hooks::located_hook(base, site.rva)
                    .map(|code| *slot = code)
                    .is_ok()
            })
        {
            let callbacks: [usize; SITES] = [
                assign_editable_template_data as *const () as usize,
                assign_template_data as *const () as usize,
                read_template_data as *const () as usize,
                destroy_template as *const () as usize,
                reset_template_data as *const () as usize,
                rebuild_template_caches as *const () as usize,
                set_foreign_equipment_permission as *const () as usize,
                set_new_equipment_permission as *const () as usize,
                set_regiment_cell as *const () as usize,
                set_regiment_cell_and_rebuild as *const () as usize,
                set_template_country as *const () as usize,
                set_template_name_group as *const () as usize,
                set_support_cell as *const () as usize,
                set_support_cell_and_rebuild as *const () as usize,
                set_template_equipment_niche as *const () as usize,
                set_template_foreign_origin as *const () as usize,
                set_template_hq_kind as *const () as usize,
                set_template_obsolete as *const () as usize,
                set_template_model_entity as *const () as usize,
                set_template_model_category as *const () as usize,
                set_regimental_support_cell as *const () as usize,
                set_regimental_support_cell_and_rebuild as *const () as usize,
                set_template_priority as *const () as usize,
                set_template_unit_type as *const () as usize,
                toggle_template_variant as *const () as usize,
                rebuild_template_ai_role as *const () as usize,
                resize_template_grids as *const () as usize,
                refresh_template_name_group as *const () as usize,
                set_template_force_recruiting as *const () as usize,
                effect_template_lock as *const () as usize,
                effect_template_cap as *const () as usize,
                effect_clear_template_cap as *const () as usize,
                effect_remove_template as *const () as usize,
                effect_country_template_lock as *const () as usize,
                set_staff_template as *const () as usize,
                set_default_garrison_template as *const () as usize,
                set_group_garrison_template as *const () as usize,
                set_state_garrison_template as *const () as usize,
                reset_occupation_status as *const () as usize,
                insert_occupation_group as *const () as usize,
                destroy_occupation_group as *const () as usize,
                destroy_occupation_status as *const () as usize,
                allocate_hq_deployment_order as *const () as usize,
                set_hq_target as *const () as usize,
                begin_hq_deployment as *const () as usize,
                cancel_hq_deployment as *const () as usize,
                destroy_hq_group as *const () as usize,
                requisition_hq_from_army as *const () as usize,
                rebuild_hq_requirements as *const () as usize,
                distribute_hq_equipment as *const () as usize,
                add_hq_manpower as *const () as usize,
                accept_hq_manpower as *const () as usize,
                take_hq_equipment as *const () as usize,
                refund_hq_equipment as *const () as usize,
                destroy_hq_deployment as *const () as usize,
                set_garrison_priority as *const () as usize,
                update_template_tracked_variant as *const () as usize,
                set_template_symbol as *const () as usize,
                add_template_effect as *const () as usize,
                add_colonial_template_effect as *const () as usize,
            ];
            for (index, &(at, original)) in located.iter().enumerate() {
                let hook = unsafe { Hook::new(at, callbacks[index], original) }
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
            collector_observers::occupation_changes::forwarder(
                collector_observers::occupation_changes::Forwarder::Templates,
                false,
            );
            collector_observers::orders_group_changes::forwarder(false);
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "native hook state change failed")?
        {
            return Ok(false);
        }
        WATCHES
            .registry
            .set_enabled(enabled && !self.hooks.is_empty());
        collector_observers::occupation_changes::forwarder(
            collector_observers::occupation_changes::Forwarder::Templates,
            enabled && !self.hooks.is_empty(),
        );
        collector_observers::orders_group_changes::forwarder(enabled && !self.hooks.is_empty());
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        collector_observers::occupation_changes::forwarder(
            collector_observers::occupation_changes::Forwarder::Templates,
            false,
        );
        collector_observers::orders_group_changes::forwarder(false);
        // SAFETY: like each Hook's relay, the watch tables are dropped only
        // once the hooks were never enabled, or were removed and drained
        // after the last capture.
        unsafe { WATCHES.registry.release() };
    }
}

fn receiver(receiver: Receiver, first: usize) -> usize {
    match receiver {
        Receiver::Data => first.saturating_sub(DATA),
        Receiver::Template | Receiver::Object => first,
        Receiver::OccupationTable => {
            first.saturating_sub(engine_mutations::template_mutations::OCCUPATION_MEMBERSHIP)
        }
        Receiver::OccupationGroup
        | Receiver::ScopeCountry
        | Receiver::CommandCountry
        | Receiver::Every => 0,
    }
}

fn selected_receiver(
    memory: &impl Memory,
    base: usize,
    kind: Receiver,
    first: usize,
    second: usize,
) -> Option<usize> {
    match kind {
        Receiver::Every => None,
        Receiver::OccupationGroup => first
            .checked_add(engine_mutations::template_mutations::OCCUPATION_PARENT)
            .and_then(|at| engine_native::read::<u64>(memory, at))
            .and_then(|at| usize::try_from(at).ok())
            .filter(|&at| at != 0),
        Receiver::ScopeCountry | Receiver::CommandCountry => (if kind == Receiver::CommandCountry {
            first.checked_add(engine_mutations::template_mutations::COMMAND_COUNTRY)
        } else {
            second.checked_add(engine_mutations::template_mutations::EFFECT_SCOPE_COUNTRY)
        })
        .and_then(|at| engine_native::read::<u32>(memory, at))
        .and_then(|raw| engine_mutations::country_input_mutations::of_link(memory, base, raw))
        .map(|country| country as usize),
        _ => Some(receiver(kind, first)).filter(|&at| at != 0),
    }
}

pub fn native_notification(
    watches: &Watches,
    site: usize,
    first: usize,
    second: usize,
) -> (Notification<'_>, Option<Notification<'static>>) {
    // SAFETY: all reads are guarded on the native calling thread.
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    notification(
        watches,
        &memory,
        BASE.load(Ordering::Acquire),
        site,
        first,
        second,
    )
}

fn notification<'a>(
    watches: &'a Watches,
    memory: &impl Memory,
    base: usize,
    site: usize,
    first: usize,
    second: usize,
) -> (Notification<'a>, Option<Notification<'static>>) {
    let selected = selected_receiver(memory, base, MUTATIONS[site].1, first, second);
    let occupation = (38..=41)
        .contains(&site)
        .then(|| collector_observers::occupation_changes::holder_changed(selected));
    let template = match selected {
        Some(at) => watches.notify(at, site),
        None => {
            watches.calls[site].fetch_add(1, Ordering::Relaxed);
            watches.registry.notify_every()
        }
    };
    (template, occupation)
}

type NativeFunction = unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize;

macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system-unwind" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = collector_timing::hook_timing::enter(
                collector_timing::hook_timing::Family::TemplateChanges,
            );
            let notification =
                ManuallyDrop::new(native_notification(&WATCHES, $site, first, second));
            let original: NativeFunction =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            let result = timing.original(|| unsafe { original(first, second, third, fourth) });
            drop(ManuallyDrop::into_inner(notification));
            result
        }
    };
}
callback!(assign_editable_template_data, 0);
callback!(assign_template_data, 1);
callback!(read_template_data, 2);
callback!(destroy_template, 3);
callback!(reset_template_data, 4);
callback!(rebuild_template_caches, 5);
callback!(set_foreign_equipment_permission, 6);
callback!(set_new_equipment_permission, 7);
callback!(set_regiment_cell, 8);
callback!(set_regiment_cell_and_rebuild, 9);
callback!(set_template_country, 10);
callback!(set_template_name_group, 11);
callback!(set_support_cell, 12);
callback!(set_support_cell_and_rebuild, 13);
callback!(set_template_equipment_niche, 14);
callback!(set_template_foreign_origin, 15);
callback!(set_template_hq_kind, 16);
callback!(set_template_obsolete, 17);
callback!(set_template_model_entity, 18);
callback!(set_template_model_category, 19);
callback!(set_regimental_support_cell, 20);
callback!(set_regimental_support_cell_and_rebuild, 21);
callback!(set_template_priority, 22);
unsafe extern "system-unwind" fn set_template_unit_type(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::TemplateChanges,
    );
    let running = ManuallyDrop::new(WATCHES.registry.running());
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let notification = ManuallyDrop::new(WATCHES.unit_type_notification(&memory, first, second));
    let original: NativeFunction =
        unsafe { std::mem::transmute(ORIGINAL[UNIT_TYPE_SITE].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(first, second, third, fourth) });
    drop(ManuallyDrop::into_inner(notification));
    drop(ManuallyDrop::into_inner(running));
    result
}
callback!(toggle_template_variant, 24);
callback!(update_template_tracked_variant, 56);
callback!(set_template_symbol, 57);
callback!(add_template_effect, 58);
callback!(add_colonial_template_effect, 59);
unsafe extern "system-unwind" fn rebuild_template_ai_role(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    use engine_native::creferenced_division_template::{AI_ROLE, AI_ROLE_FALLBACK};
    let timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::TemplateChanges,
    );
    let running = ManuallyDrop::new(WATCHES.registry.running());
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let fields = first.checked_add(AI_ROLE - DATA);
    let read = || {
        fields
            .and_then(|at| engine_native::read::<[u8; AI_ROLE_FALLBACK + 4 - AI_ROLE]>(&memory, at))
    };
    let before = read();
    let original: NativeFunction =
        unsafe { std::mem::transmute(ORIGINAL[AI_ROLE_SITE].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(first, second, third, fourth) });
    if before.is_some() && before == read() {
        WATCHES.calls[AI_ROLE_SITE].fetch_add(1, Ordering::Relaxed);
        WATCHES
            .unchanged_ai_role_rebuilds
            .fetch_add(1, Ordering::Relaxed);
    } else {
        drop(native_notification(&WATCHES, AI_ROLE_SITE, first, second));
    }
    drop(ManuallyDrop::into_inner(running));
    result
}
callback!(resize_template_grids, 26);
callback!(refresh_template_name_group, 27);
callback!(set_template_force_recruiting, 28);
callback!(effect_template_lock, 29);
callback!(effect_template_cap, 30);
callback!(effect_clear_template_cap, 31);
callback!(effect_remove_template, 32);
callback!(effect_country_template_lock, 33);
callback!(set_staff_template, 34);
callback!(set_default_garrison_template, 35);
callback!(set_group_garrison_template, 36);
callback!(set_state_garrison_template, 37);
callback!(reset_occupation_status, 38);
unsafe extern "system-unwind" fn insert_occupation_group(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
    fifth: usize,
) -> usize {
    let timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::TemplateChanges,
    );
    let notification = ManuallyDrop::new(native_notification(&WATCHES, 39, first, second));
    let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[39].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(first, second, third, fourth, fifth) });
    drop(ManuallyDrop::into_inner(notification));
    result
}
callback!(destroy_occupation_group, 40);
callback!(destroy_occupation_status, 41);
callback!(allocate_hq_deployment_order, 42);
callback!(set_hq_target, 43);
callback!(begin_hq_deployment, 44);
callback!(cancel_hq_deployment, 45);
unsafe extern "system-unwind" fn destroy_hq_group(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::TemplateChanges,
    );
    let notification = ManuallyDrop::new((
        native_notification(&WATCHES, 46, first, second),
        collector_observers::orders_group_changes::forwarded(first),
        collector_observers::order_membership_changes::group_changing(first),
    ));
    let original: NativeFunction =
        unsafe { std::mem::transmute(ORIGINAL[46].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(first, second, third, fourth) });
    drop(ManuallyDrop::into_inner(notification));
    result
}
unsafe extern "system-unwind" fn requisition_hq_from_army(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::TemplateChanges,
    );
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let state = || {
        engine_native::read::<u64>(
            &memory,
            first.wrapping_add(engine_native::template_hq_context::HOLDER),
        )
        .and_then(|holder| usize::try_from(holder).ok())
        .and_then(|holder| {
            if holder == 0 {
                Some((0, 0))
            } else {
                engine_native::read::<u8>(
                    &memory,
                    holder.wrapping_add(engine_native::template_hq_context::REQUISITIONED),
                )
                .map(|flag| (holder, flag))
            }
        })
    };
    let running = ManuallyDrop::new(WATCHES.registry.running());
    let before = state();
    let original: NativeFunction =
        unsafe { std::mem::transmute(ORIGINAL[47].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(first, second, third, fourth) });
    if before.is_none() || before != state() {
        drop(native_notification(&WATCHES, 47, first, second));
    }
    drop(ManuallyDrop::into_inner(running));
    result
}

unsafe extern "system-unwind" fn rebuild_hq_requirements(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::TemplateChanges,
    );
    let template = second.wrapping_sub(engine_native::template_hq_context::TEMPLATE_REQUIREMENTS);
    let running = ManuallyDrop::new(WATCHES.registry.running());
    let before = HqSource {
        source: second,
        requested: third as u32,
        template: running.dependency_stamp(template),
        holder: running.dependency_stamp(first),
    };
    let unchanged = HQ_SOURCES
        .try_lock()
        .ok()
        .and_then(|sources| {
            sources
                .binary_search_by_key(&first, |&(holder, _)| holder)
                .ok()
                .map(|at| sources[at].1)
        })
        .is_some_and(|held| same_hq_source(held, before));
    let notification = ManuallyDrop::new((!unchanged).then(|| WATCHES.notify(first, 48)));
    let original: NativeFunction =
        unsafe { std::mem::transmute(ORIGINAL[48].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(first, second, third, fourth) });
    drop(ManuallyDrop::into_inner(notification));
    let after_template = running.dependency_stamp(template);
    let dirty = !unchanged || before.template.is_none() || before.template != after_template;
    if unchanged && dirty {
        drop(WATCHES.notify(first, 48));
    }
    let after_holder = running.dependency_stamp(first);
    let expected_holder = before.holder.and_then(|(epoch, sequence)| {
        sequence
            .checked_add(u64::from(dirty))
            .map(|sequence| (epoch, sequence))
    });
    if before.template.is_some()
        && before.template == after_template
        && expected_holder.is_some()
        && expected_holder == after_holder
        && let Ok(mut sources) = HQ_SOURCES.try_lock()
        && let Ok(at) = sources.binary_search_by_key(&first, |&(holder, _)| holder)
    {
        sources[at].1 = HqSource {
            holder: after_holder,
            ..before
        };
    }
    drop(ManuallyDrop::into_inner(running));
    result
}
fn equipment_delivered(available: i64, remaining: Option<i64>) -> bool {
    remaining.is_none_or(|remaining| remaining < available)
}

unsafe extern "system-unwind" fn distribute_hq_equipment(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::TemplateChanges,
    );
    let running = ManuallyDrop::new(WATCHES.registry.running());
    let original: NativeFunction =
        unsafe { std::mem::transmute(ORIGINAL[49].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(first, second, third, fourth) });
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    if equipment_delivered(fourth as i64, engine_native::read::<i64>(&memory, second)) {
        drop(WATCHES.notify(first, 49));
    }
    drop(ManuallyDrop::into_inner(running));
    result
}

unsafe extern "system-unwind" fn add_hq_manpower(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::TemplateChanges,
    );
    let notification = ManuallyDrop::new(((second as i32) > 0).then(|| WATCHES.notify(first, 50)));
    let original: NativeFunction =
        unsafe { std::mem::transmute(ORIGINAL[50].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(first, second, third, fourth) });
    drop(ManuallyDrop::into_inner(notification));
    result
}

unsafe extern "system-unwind" fn accept_hq_manpower(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
) -> usize {
    let timing = collector_timing::hook_timing::enter(
        collector_timing::hook_timing::Family::TemplateChanges,
    );
    let running = ManuallyDrop::new(WATCHES.registry.running());
    let original: NativeFunction =
        unsafe { std::mem::transmute(ORIGINAL[51].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(first, second, third, fourth) });
    if (result as i32) > 0 {
        drop(WATCHES.notify(first, 51));
    }
    drop(ManuallyDrop::into_inner(running));
    result
}
callback!(take_hq_equipment, 52);
callback!(refund_hq_equipment, 53);
callback!(destroy_hq_deployment, 54);
callback!(set_garrison_priority, 55);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_type_stores_preserve_quiet_only_for_a_readable_unchanged_word() {
        struct Word {
            address: Option<usize>,
            value: Option<u32>,
            reads: std::cell::Cell<usize>,
        }
        impl Memory for Word {
            fn copy(&self, at: usize, output: &mut [u8]) -> bool {
                self.reads.set(self.reads.get() + 1);
                assert_eq!(Some(at), self.address);
                assert_eq!(output.len(), 4);
                let Some(value) = self.value else {
                    return false;
                };
                output.copy_from_slice(&value.to_le_bytes());
                true
            }
        }
        for (data, before, unchanged) in [
            (0x1000 + DATA, Some(7), true),
            (0x1000 + DATA, Some(8), false),
            (0x1000 + DATA, None, false),
            (usize::MAX, Some(7), false),
            (0, Some(7), false),
            (DATA - 1, Some(7), false),
            (DATA, Some(7), false),
        ] {
            let watches = Watches::enabled(4);
            let template = data.checked_sub(DATA).filter(|&pointer| pointer != 0);
            let receiver = template.unwrap_or(0x1000);
            let sequence = watches.registry.sequence(receiver).unwrap();
            let other = watches.registry.sequence(0x2000).unwrap();
            let quiet = watches.registry.quiet().unwrap();
            let memory = Word {
                address: data.checked_add(
                    engine_native::creferenced_division_template::UNIT_TYPE_OVERRIDE - DATA,
                ),
                value: before,
                reads: Default::default(),
            };
            let running = watches.registry.running();
            collector_test_support::with_allocations_denied(|| {
                let notification =
                    watches.unit_type_notification(&memory, data, 0xabcd_0000_0000_0007);
                assert_eq!(notification.is_none(), unchanged);
                assert!(watches.registry.quiet().is_none());
                assert!(watches.registry.sequence(receiver).is_none());
                drop(notification);
            });
            assert!(watches.registry.quiet().is_none());
            drop(running);
            assert_eq!(
                watches.registry.sequence(receiver) == Some(sequence),
                unchanged
            );
            assert_eq!(watches.registry.quiet() == Some(quiet), unchanged);
            assert_eq!(
                watches.registry.sequence(0x2000) == Some(other),
                template.is_some()
            );
            assert_eq!(watches.calls[UNIT_TYPE_SITE].load(Ordering::Relaxed), 1);
            assert_eq!(
                watches.unchanged_unit_type_writes.load(Ordering::Relaxed),
                u64::from(unchanged)
            );
            assert_eq!(
                memory.reads.get(),
                usize::from(template.is_some() && memory.address.is_some())
            );
        }
    }

    #[test]
    fn unit_type_callback_preserves_arguments_return_and_in_flight_exclusion() {
        const TEST: &str = "template_changes::tests::unit_type_callback_preserves_arguments_return_and_in_flight_exclusion";
        const CHILD: &str = "BATTLEPLAN_TEMPLATE_UNIT_TYPE_CALLBACK_TEST";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST, "--nocapture"])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        unsafe extern "system-unwind" fn original(a: usize, b: usize, c: usize, d: usize) -> usize {
            assert_eq!([c, d], [0x1234_5678_9abc_def0, usize::MAX - 11]);
            assert!(WATCHES.registry.quiet().is_none());
            assert!(WATCHES.registry.sequence(a - DATA).is_none());
            unsafe {
                std::ptr::write_unaligned(
                    (a + engine_native::creferenced_division_template::UNIT_TYPE_OVERRIDE - DATA)
                        as *mut u32,
                    b as u32,
                );
            }
            a.rotate_left(7) ^ b.rotate_left(19) ^ c.rotate_left(31) ^ d.rotate_left(43)
        }
        struct Restore(usize);
        impl Drop for Restore {
            fn drop(&mut self) {
                ORIGINAL[UNIT_TYPE_SITE].store(self.0, Ordering::Release);
            }
        }
        let _restore = Restore(
            ORIGINAL[UNIT_TYPE_SITE].swap(original as *const () as usize, Ordering::AcqRel),
        );
        let mut bytes = [0; engine_native::creferenced_division_template::UNIT_TYPE_OVERRIDE + 4];
        bytes[engine_native::creferenced_division_template::UNIT_TYPE_OVERRIDE..]
            .copy_from_slice(&7_u32.to_le_bytes());
        let pointer = bytes.as_mut_ptr() as usize;
        WATCHES.registry.offer(4);
        WATCHES.registry.boundary(false);
        WATCHES.registry.set_enabled(true);
        for (value, unchanged) in [(0xabcd_0000_0000_0007, true), (9, false)] {
            let quiet = WATCHES.registry.quiet().unwrap();
            let sequence = WATCHES.registry.sequence(pointer).unwrap();
            let a = pointer + DATA;
            let c = 0x1234_5678_9abc_def0_usize;
            let d = usize::MAX - 11;
            assert_eq!(
                unsafe { set_template_unit_type(a, value, c, d) },
                a.rotate_left(7) ^ value.rotate_left(19) ^ c.rotate_left(31) ^ d.rotate_left(43)
            );
            assert_eq!(WATCHES.registry.quiet() == Some(quiet), unchanged);
            assert_eq!(
                WATCHES.registry.sequence(pointer) == Some(sequence),
                unchanged
            );
        }
        assert_eq!(WATCHES.calls[UNIT_TYPE_SITE].load(Ordering::Relaxed), 2);
        assert_eq!(
            WATCHES.unchanged_unit_type_writes.load(Ordering::Relaxed),
            1
        );
    }

    #[test]
    fn occupation_holder_callbacks_forward_real_certificates_and_template_admission() {
        use collector_observers::occupation_changes as occupation;
        const TEST: &str = "template_changes::tests::occupation_holder_callbacks_forward_real_certificates_and_template_admission";
        const CHILD: &str = "BATTLEPLAN_OCCUPATION_TEMPLATE_FORWARDING_TEST";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", TEST, "--nocapture"])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        const HOLDER: usize = 0x4000;
        const UNRELATED: usize = 0x5000;
        unsafe extern "system-unwind" fn four(a: usize, b: usize, c: usize, d: usize) -> usize {
            assert_eq!([b, c, d], [2, 3, 4]);
            assert!(occupation::active() > 0);
            assert!(occupation::quiet().is_none());
            assert!(occupation::sequence(HOLDER).is_none());
            assert!(WATCHES.registry.quiet().is_none());
            a.rotate_left(7) ^ b.rotate_left(19) ^ c.rotate_left(31) ^ d.rotate_left(43)
        }
        unsafe extern "system-unwind" fn five(
            a: usize,
            b: usize,
            c: usize,
            d: usize,
            e: usize,
        ) -> usize {
            assert_eq!(e, 5);
            (unsafe { four(a, b, c, d) }) ^ e.rotate_left(53)
        }
        struct Restore([(usize, usize); 4]);
        impl Drop for Restore {
            fn drop(&mut self) {
                for (site, original) in self.0 {
                    ORIGINAL[site].store(original, Ordering::Release);
                }
            }
        }
        let _originals = Restore([38, 39, 40, 41].map(|site| {
            let callback = if site == 39 {
                five as *const () as usize
            } else {
                four as *const () as usize
            };
            (site, ORIGINAL[site].swap(callback, Ordering::AcqRel))
        }));
        WATCHES.registry.offer(16);
        WATCHES.registry.boundary(false);
        WATCHES.registry.set_enabled(true);
        occupation::demand(16);
        occupation::prepare();
        occupation::boundary(true);
        occupation::observe_for_tests(true);
        for forwarder in [
            occupation::Forwarder::Countries,
            occupation::Forwarder::States,
            occupation::Forwarder::Territory,
            occupation::Forwarder::Relations,
            occupation::Forwarder::Exile,
        ] {
            occupation::forwarder(forwarder, true);
        }
        assert!(
            occupation::quiet().is_none(),
            "templates have not admitted their writers"
        );
        assert!(occupation::sequence(HOLDER).is_none());
        occupation::forwarder(occupation::Forwarder::Templates, true);
        assert!(occupation::quiet().is_some());

        let mut group = [0_usize; 2];
        group[engine_mutations::template_mutations::OCCUPATION_PARENT / size_of::<usize>()] =
            HOLDER;
        let group_pointer = group.as_ptr() as usize;
        for (site, first) in [
            (38, HOLDER),
            (
                39,
                HOLDER + engine_mutations::template_mutations::OCCUPATION_MEMBERSHIP,
            ),
            (40, group_pointer),
            (41, HOLDER),
        ] {
            let holder = occupation::sequence(HOLDER).unwrap();
            let unrelated = occupation::sequence(UNRELATED).unwrap();
            let quiet = occupation::quiet().unwrap();
            let expected = first.rotate_left(7)
                ^ 2_usize.rotate_left(19)
                ^ 3_usize.rotate_left(31)
                ^ 4_usize.rotate_left(43);
            let result = unsafe {
                match site {
                    38 => reset_occupation_status(first, 2, 3, 4),
                    39 => insert_occupation_group(first, 2, 3, 4, 5),
                    40 => destroy_occupation_group(first, 2, 3, 4),
                    41 => destroy_occupation_status(first, 2, 3, 4),
                    _ => unreachable!(),
                }
            };
            assert_eq!(
                result,
                expected
                    ^ if site == 39 {
                        5_usize.rotate_left(53)
                    } else {
                        0
                    }
            );
            assert_ne!(occupation::sequence(HOLDER), Some(holder));
            assert_eq!(occupation::sequence(UNRELATED), Some(unrelated));
            assert_ne!(occupation::quiet(), Some(quiet));
        }

        group[engine_mutations::template_mutations::OCCUPATION_PARENT / size_of::<usize>()] = 0;
        for first in [group.as_ptr() as usize, 0] {
            let holder = occupation::sequence(HOLDER).unwrap();
            let unrelated = occupation::sequence(UNRELATED).unwrap();
            unsafe { destroy_occupation_group(first, 2, 3, 4) };
            assert_ne!(occupation::sequence(HOLDER), Some(holder));
            assert_ne!(occupation::sequence(UNRELATED), Some(unrelated));
        }

        let before_disabled = occupation::sequence(HOLDER).unwrap();
        let mut hooks = Hooks { hooks: Vec::new() };
        assert!(hooks.set_enabled(false).unwrap());
        assert!(occupation::quiet().is_none());
        assert!(occupation::sequence(HOLDER).is_none());
        occupation::forwarder(occupation::Forwarder::Templates, true);
        assert!(occupation::quiet().is_some());
        assert_ne!(occupation::sequence(HOLDER), Some(before_disabled));
        drop(hooks);
        assert!(occupation::quiet().is_none());
        assert!(occupation::sequence(HOLDER).is_none());
        occupation::observe_for_tests(false);
    }

    #[test]
    fn equipment_distribution_notifies_only_a_delivery_or_unreadable_result() {
        assert!(!equipment_delivered(17, Some(17)));
        assert!(!equipment_delivered(0, Some(0)));
        assert!(!equipment_delivered(-17, Some(0)));
        assert!(equipment_delivered(17, Some(16)));
        assert!(equipment_delivered(17, None));
    }

    #[test]
    fn equal_hourly_hq_rebuild_inputs_do_not_revoke_pool_carry() {
        let source = HqSource {
            source: 0x5000,
            requested: 17,
            template: Some((1, 9)),
            holder: Some((1, 11)),
        };
        assert!(same_hq_source(source, source));
        for changed in [
            HqSource {
                requested: 18,
                ..source
            },
            HqSource {
                source: 0x6000,
                ..source
            },
            HqSource {
                template: Some((1, 10)),
                ..source
            },
            HqSource {
                holder: Some((1, 12)),
                ..source
            },
            HqSource {
                template: None,
                holder: None,
                ..source
            },
        ] {
            assert!(!same_hq_source(source, changed));
        }
    }

    #[test]
    fn template_callbacks_preserve_register_arguments_stack_argument_and_full_return() {
        unsafe extern "system-unwind" fn four(a: usize, b: usize, c: usize, d: usize) -> usize {
            a.rotate_left(7) ^ b.rotate_left(19) ^ c.rotate_left(31) ^ d.rotate_left(43)
        }
        unsafe extern "system-unwind" fn five(
            a: usize,
            b: usize,
            c: usize,
            d: usize,
            e: usize,
        ) -> usize {
            (unsafe { four(a, b, c, d) }) ^ e.rotate_left(53)
        }
        struct Restore(usize, usize);
        impl Drop for Restore {
            fn drop(&mut self) {
                ORIGINAL[self.0].store(self.1, Ordering::Release);
            }
        }
        let _four = Restore(
            43,
            ORIGINAL[43].swap(four as *const () as usize, Ordering::AcqRel),
        );
        let _five = Restore(
            39,
            ORIGINAL[39].swap(five as *const () as usize, Ordering::AcqRel),
        );
        let args = [
            0x1234,
            usize::MAX - 17,
            0xabcd_9876_1234_4567,
            0x9876_5432_ffff_0001,
            0xfedc_ba98_7654_3210,
        ];
        unsafe {
            assert_eq!(
                set_hq_target(args[0], args[1], args[2], args[3]),
                four(args[0], args[1], args[2], args[3])
            );
            assert_eq!(
                insert_occupation_group(args[0], args[1], args[2], args[3], args[4]),
                five(args[0], args[1], args[2], args[3], args[4])
            );
        }
    }

    #[test]
    fn a_writer_moves_only_its_templates_certificate_and_the_quiet_token() {
        let watches = Watches::enabled(16);
        let [first, second] =
            [0x1000, 0x2000].map(|template| Changes::sequence(&watches, template).unwrap());
        let quiet = Changes::quiet(&watches).unwrap();
        let named = receiver(MUTATIONS[0].1, 0x2000 + DATA);
        assert_eq!(named, 0x2000);
        drop(watches.notify(named, 0));
        assert_eq!(Changes::sequence(&watches, 0x1000), Some(first));
        assert_ne!(Changes::sequence(&watches, 0x2000), Some(second));
        assert_ne!(Changes::quiet(&watches), Some(quiet));
        assert_eq!(watches.calls[0].load(Ordering::Relaxed), 1);
        assert_eq!(receiver(MUTATIONS[3].1, 0x1000), 0x1000);
        let running = watches.notify(0x1000, 3);
        assert!(Changes::sequence(&watches, 0x1000).is_none());
        assert!(Changes::quiet(&watches).is_none());
        drop(running);
        assert!(!watches.poisoned());
    }

    #[test]
    fn the_custom_icon_command_revokes_every_template_certificate() {
        let site = MUTATIONS
            .iter()
            .position(|(site, _)| site.name == "set_template_symbol")
            .unwrap();
        assert_eq!(MUTATIONS[site].1, Receiver::Every);
        let watches = Watches::enabled(16);
        let receivers = [0x1000, 0x2000, 7];
        let before = receivers.map(|receiver| Changes::sequence(&watches, receiver).unwrap());
        let quiet = Changes::quiet(&watches).unwrap();
        let (running, occupation) = native_notification(&watches, site, 0xc0de, 0);
        assert!(occupation.is_none());
        assert!(Changes::sequence(&watches, 0x1000).is_none());
        drop(running);
        for (receiver, before) in receivers.into_iter().zip(before) {
            assert_ne!(Changes::sequence(&watches, receiver), Some(before));
        }
        assert_ne!(Changes::quiet(&watches), Some(quiet));
        assert_eq!(watches.calls[site].load(Ordering::Relaxed), 1);
        assert!(!watches.poisoned());
    }

    #[test]
    fn scripted_template_additions_revoke_only_their_scope_countrys_templates() {
        use engine_mutations::template_mutations::EFFECT_SCOPE_COUNTRY;
        use engine_native::{ccountry, cgame_state, memory::SliceMemory};
        const START: usize = 0x1000_0000;
        let base = START - cgame_state::SINGLETON_RVA.0;
        let state = START + 0x100;
        let countries = START + 0x800;
        let links = START + 0x900;
        let country = START + 0x1000;
        let scope = START + 0x2000;
        let unlinked = START + 0x2100;
        let mut bytes = vec![0_u8; 0x3000];
        let mut put = |address: usize, value: &[u8]| {
            let at = address - START;
            bytes[at..at + value.len()].copy_from_slice(value);
        };
        let mut array = |address: usize, data: usize, count: u32| {
            put(address, &(data as u64).to_le_bytes());
            put(address + 8, &count.to_le_bytes());
            put(address + 12, &count.to_le_bytes());
        };
        array(state + cgame_state::COUNTRIES, countries, 2);
        array(state + cgame_state::COUNTRY_LINKS, links, 16);
        put(START, &(state as u64).to_le_bytes());
        put(countries + 8, &(country as u64).to_le_bytes());
        put(links + 7 * 4, &1_u32.to_le_bytes());
        put(country + ccountry::ID, &7_u32.to_le_bytes());
        put(scope + EFFECT_SCOPE_COUNTRY, &7_u32.to_le_bytes());
        put(unlinked + EFFECT_SCOPE_COUNTRY, &11_u32.to_le_bytes());
        let memory = SliceMemory {
            base: START,
            bytes: &bytes,
        };
        for name in ["add_template_effect", "add_colonial_template_effect"] {
            let site = MUTATIONS
                .iter()
                .position(|(site, _)| site.name == name)
                .unwrap();
            assert_eq!(MUTATIONS[site].1, Receiver::ScopeCountry, "{name}");
            let watches = Watches::enabled(16);
            let receivers = [7, 9, 0x5000, 0x6000];
            let before = receivers.map(|receiver| Changes::sequence(&watches, receiver).unwrap());
            let (running, occupation) = notification(&watches, &memory, base, site, 0xe0, scope);
            assert!(occupation.is_none());
            assert!(Changes::sequence(&watches, 7).is_none(), "{name}");
            drop(running);
            let after = receivers.map(|receiver| Changes::sequence(&watches, receiver));
            assert_ne!(after[0], Some(before[0]), "{name}");
            for index in 1..receivers.len() {
                assert_eq!(after[index], Some(before[index]), "{name}");
            }
            assert_eq!(watches.calls[site].load(Ordering::Relaxed), 1);
            drop(notification(&watches, &memory, base, site, 0xe0, unlinked));
            assert!(
                receivers[1..]
                    .iter()
                    .zip(&before[1..])
                    .all(|(&receiver, &before)| Changes::sequence(&watches, receiver)
                        != Some(before)),
                "{name}"
            );
            assert!(!watches.poisoned());
        }
    }
}
