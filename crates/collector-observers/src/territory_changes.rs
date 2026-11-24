use crate::{
    dirty_set::{Dirty, DirtySet},
    hook::Hook,
};
use engine_mutations::{territory_mutations::MUTATIONS, territory_mutations::Receiver};
use engine_native_memory::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const SITES: usize = MUTATIONS.len();
static DOMAIN: Domain = Domain::new();
static ORIGINAL: [AtomicUsize; SITES] = [const { AtomicUsize::new(0) }; SITES];
static READS: [[AtomicU64; 4]; 2] = [const { [const { AtomicU64::new(0) }; 4] }; 2];
static SLOTS: [AtomicUsize; 2] = [const { AtomicUsize::new(0) }; 2];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Table {
    States = 0,
    Provinces = 1,
}

pub struct Domain {
    provinces: DirtySet,
    states: DirtySet,
    province_states: DirtySet,
    modifier_states: DirtySet,
    calls: [AtomicU64; SITES],
    unnamed: AtomicU64,
    buildings: [AtomicU64; 2],
}

#[must_use]
pub struct Marks<'a>(#[expect(dead_code, reason = "held only so its drop releases the dirty sets")] [Option<Dirty<'a>>; 2]);

impl Default for Domain {
    fn default() -> Self {
        Self::new()
    }
}

impl Domain {
    pub const fn new() -> Self {
        Self {
            provinces: DirtySet::new(),
            states: DirtySet::new(),
            province_states: DirtySet::new(),
            modifier_states: DirtySet::new(),
            calls: [const { AtomicU64::new(0) }; SITES],
            unnamed: AtomicU64::new(0),
            buildings: [const { AtomicU64::new(0) }; 2],
        }
    }

    fn sets(&self) -> [&DirtySet; 4] {
        [
            &self.provinces,
            &self.states,
            &self.province_states,
            &self.modifier_states,
        ]
    }

    pub fn modifier_state(&self, id: Option<usize>) -> Dirty<'_> {
        self.mark(&self.modifier_states, id)
    }

    fn mark<'a>(&'a self, set: &'a DirtySet, id: Option<usize>) -> Dirty<'a> {
        match id {
            Some(id) => set.notify(id),
            None => {
                self.unnamed.fetch_add(1, Ordering::Relaxed);
                set.notify_every()
            }
        }
    }

    pub fn notify(&self, site: usize, target: Target) -> Marks<'_> {
        self.calls[site].fetch_add(1, Ordering::Relaxed);
        Marks(match target {
            Target::Province(id) => [Some(self.mark(&self.provinces, id)), None],
            Target::State {
                id,
                state,
                provinces,
            } => [
                state.then(|| self.mark(&self.states, id)),
                provinces.then(|| self.mark(&self.province_states, id)),
            ],
            Target::EveryProvince => [Some(self.provinces.notify_every()), None],
            Target::EveryState => [Some(self.states.notify_every()), None],
        })
    }

    pub fn building(&self, flags: u32, location: u32) -> Marks<'_> {
        use engine_native::cprovince::buildings::{PROVINCE_FLAG, STATE_FLAG};
        self.buildings[0].fetch_add(1, Ordering::Relaxed);
        let id = (location != 0).then_some(location as usize);
        Marks([
            (flags & PROVINCE_FLAG != 0).then(|| self.mark(&self.provinces, id)),
            (flags & STATE_FLAG != 0).then(|| self.mark(&self.states, id)),
        ])
    }

    pub fn building_unnamed(&self) -> Marks<'_> {
        self.buildings[1].fetch_add(1, Ordering::Relaxed);
        Marks([
            Some(self.provinces.notify_every()),
            Some(self.states.notify_every()),
        ])
    }

    fn poison(&self) {
        for set in self.sets() {
            set.poison();
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn enabled(states: usize, provinces: usize) -> Self {
        let domain = Self::new();
        domain.provinces.offer(provinces);
        domain.states.offer(states);
        domain.province_states.offer(states);
        domain.modifier_states.offer(states);
        for set in domain.sets() {
            set.boundary(false);
            set.set_enabled(true);
        }
        domain
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn boundary(&self, invalidate: bool) {
        for set in self.sets() {
            set.boundary(invalidate);
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn poisoned(&self) -> bool {
        self.sets().iter().any(|set| set.poisoned())
    }
}

pub trait Changes: Sync {
    fn take_states(&self, visit: &mut dyn FnMut(usize)) -> Option<u64>;
    fn take_provinces(
        &self,
        provinces: &mut dyn FnMut(usize),
        states: &mut dyn FnMut(usize),
    ) -> Option<[u64; 2]>;
    fn take_modifier_states(&self, visit: &mut dyn FnMut(usize)) -> Option<u64>;
}

impl Changes for Domain {
    fn take_states(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        self.states.take(visit)
    }
    fn take_provinces(
        &self,
        provinces: &mut dyn FnMut(usize),
        states: &mut dyn FnMut(usize),
    ) -> Option<[u64; 2]> {
        let provinces = self.provinces.take(provinces);
        let states = self.province_states.take(states);
        Some([provinces?, states?])
    }
    fn take_modifier_states(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        self.modifier_states.take(visit)
    }
}

pub struct Observer;
impl Changes for Observer {
    fn take_states(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        DOMAIN.take_states(visit)
    }
    fn take_provinces(
        &self,
        provinces: &mut dyn FnMut(usize),
        states: &mut dyn FnMut(usize),
    ) -> Option<[u64; 2]> {
        DOMAIN.take_provinces(provinces, states)
    }
    fn take_modifier_states(&self, visit: &mut dyn FnMut(usize)) -> Option<u64> {
        DOMAIN.take_modifier_states(visit)
    }
}

static SCRIPTED_VP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SCRIPTED_CONTROLLERS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn scripted_capture(victory_point: bool, controllers: bool) {
    SCRIPTED_VP.store(victory_point, Ordering::Relaxed);
    SCRIPTED_CONTROLLERS.store(controllers, Ordering::Relaxed);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    Province(Option<usize>),
    State {
        id: Option<usize>,
        state: bool,
        provinces: bool,
    },
    EveryProvince,
    EveryState,
}

fn object_id(object: usize, offset: usize) -> Option<usize> {
    let memory = unsafe { engine_native_memory::memory::LiveMemory::new() };
    object
        .checked_add(offset)
        .and_then(|address| engine_native_memory::read::<u32>(&memory, address))
        .filter(|&id| id != 0)
        .map(|id| id as usize)
}

pub fn target(receiver: Receiver, first: usize) -> Target {
    match receiver {
        Receiver::Province => Target::Province(object_id(first, engine_native::cprovince::ID)),
        Receiver::State(rows) => Target::State {
            id: object_id(first, engine_native::cstate::ID),
            state: rows.state,
            provinces: rows.provinces,
        },
        Receiver::EveryProvince => Target::EveryProvince,
        Receiver::EveryState => Target::EveryState,
    }
}

pub fn active() -> usize {
    DOMAIN.sets().iter().map(|set| set.active()).sum()
}

pub fn boundary(invalidate: bool) {
    for set in DOMAIN.sets() {
        set.boundary(invalidate);
    }
}

pub fn poison() {
    DOMAIN.poison();
}

pub fn building_changed(flags: u32, location: u32) -> Marks<'static> {
    DOMAIN.building(flags, location)
}

pub fn building_unnamed() -> Marks<'static> {
    DOMAIN.building_unnamed()
}

pub fn modifier_state(id: Option<usize>) -> Dirty<'static> {
    DOMAIN.modifier_state(id)
}

static HOOKS_ENABLED: AtomicBool = AtomicBool::new(false);
static MODIFIER_FORWARDER: AtomicBool = AtomicBool::new(false);

pub fn modifier_forwarder(enabled: bool) {
    MODIFIER_FORWARDER.store(enabled, Ordering::SeqCst);
    observe_modifiers();
}

fn observe_modifiers() {
    DOMAIN.modifier_states.set_enabled(
        HOOKS_ENABLED.load(Ordering::SeqCst) && MODIFIER_FORWARDER.load(Ordering::SeqCst),
    );
}

pub fn presize() {
    let [states, provinces] = SLOTS.each_ref().map(|slots| slots.load(Ordering::Relaxed));
    if states != 0 {
        DOMAIN.states.offer(states);
        DOMAIN.province_states.offer(states);
        DOMAIN.modifier_states.offer(states);
    }
    if provinces != 0 {
        DOMAIN.provinces.offer(provinces);
    }
}

pub fn state_slots() -> usize {
    SLOTS[Table::States as usize].load(Ordering::Relaxed)
}

pub fn walked_provinces(slots: usize) {
    SLOTS[Table::Provinces as usize].store(slots, Ordering::Relaxed);
}

pub fn collect() {
    for set in DOMAIN.sets() {
        set.collect();
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reused: u64,
    pub read: u64,
    pub quiet: bool,
    pub full: bool,
}

pub fn record(table: Table, counts: &Counts, slots: Option<usize>) {
    for (target, value) in READS[table as usize].iter().zip([
        counts.reused,
        counts.read,
        u64::from(counts.quiet),
        u64::from(counts.full),
    ]) {
        target.fetch_add(value, Ordering::Relaxed);
    }
    if let Some(slots) = slots {
        SLOTS[table as usize].store(slots, Ordering::Relaxed);
    }
}


pub struct Hooks {
    hooks: Vec<Hook>,
}
impl Hooks {
    /// The runtime's build gate admitted this family before construction.
    /// A differing sentinel leaves the family uninstalled; both tables then
    /// read every row afresh.
    ///
    /// # Safety
    /// The memory source and base must identify the authenticated executable in
    /// this process. The caller owns this family exclusively and must restore
    /// its hooks and quiesce native calls before releasing code or unloading.
    pub unsafe fn new(_memory: &impl Memory, base: usize) -> Result<Self, &'static str> {
        let mut hooks = Vec::new();
        if cfg!(feature = "territory_changes")
            && MUTATIONS.iter().enumerate().all(|(index, (site, _))| {
                (index == 12 && !SCRIPTED_CONTROLLERS.load(Ordering::Relaxed))
                    || (index == 13 && !SCRIPTED_VP.load(Ordering::Relaxed))
                    || crate::verified(base, site.rva).is_ok()
            })
        {
            let callbacks: [usize; SITES] = [
                province_controller as *const () as usize,
                victory_points as *const () as usize,
                read_province as *const () as usize,
                insert_border_conflict as *const () as usize,
                erase_border_conflict as *const () as usize,
                rebuild_owner_areas as *const () as usize,
                initialize_provinces as *const () as usize,
                state_owner_entry as *const () as usize,
                state_controller as *const () as usize,
                reset_state as *const () as usize,
                read_state as *const () as usize,
                state_name_effect as *const () as usize,
                recount_single_controller as *const () as usize,
                refresh_cached_victory_point as *const () as usize,
                add_contested_owner as *const () as usize,
                remove_contested_owner as *const () as usize,
            ];
            for (index, (site, _)) in MUTATIONS.iter().enumerate() {
                if (index == 12 && !SCRIPTED_CONTROLLERS.load(Ordering::Relaxed))
                    || (index == 13 && !SCRIPTED_VP.load(Ordering::Relaxed))
                {
                    continue;
                }
                let code = engine_sites::hook(base, site.rva)?;
                let hook =
                    unsafe { Hook::new(code.address, callbacks[index], code.sentinel::<16>()?) }
                        .map_err(|_| "native hook construction failed")?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            crate::occupation_changes::forwarder(
                crate::occupation_changes::Forwarder::Territory,
                false,
            );
            HOOKS_ENABLED.store(false, Ordering::SeqCst);
            for set in DOMAIN.sets() {
                set.set_enabled(false);
            }
            crate::country_input_changes::forwarder(
                crate::country_input_changes::Forwarder::Territory,
                false,
            );
            crate::event_feed_observer::forwarder(
                crate::event_feed_observer::Forwarder::Territory,
                false,
            );
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "native hook state change failed")?
        {
            return Ok(false);
        }
        let [provinces, states, province_states, _] = DOMAIN.sets();
        for set in [provinces, states, province_states] {
            set.set_enabled(enabled && !self.hooks.is_empty());
        }
        HOOKS_ENABLED.store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        observe_modifiers();
        crate::country_input_changes::forwarder(
            crate::country_input_changes::Forwarder::Territory,
            enabled && !self.hooks.is_empty(),
        );
        crate::event_feed_observer::forwarder(
            crate::event_feed_observer::Forwarder::Territory,
            enabled && !self.hooks.is_empty(),
        );
        crate::occupation_changes::forwarder(
            crate::occupation_changes::Forwarder::Territory,
            enabled && !self.hooks.is_empty(),
        );
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        // SAFETY: like each Hook's relay, the bitmaps are dropped only once
        // the hooks were never enabled, or were removed and drained after
        // the last capture.
        for set in DOMAIN.sets() {
            unsafe { set.release() };
        }
    }
}

macro_rules! callback {
    ($name:ident, $site:expr) => {
        unsafe extern "system" fn $name(
            first: usize,
            second: usize,
            third: usize,
            fourth: usize,
        ) -> usize {
            let timing = crate::hook_timing::enter(crate::hook_timing::Family::TerritoryChanges);
            let _occupation = (7..=11).contains(&$site).then(|| {
                crate::occupation_changes::changed_pair(
                    first,
                    first.wrapping_add(engine_native::cstate::COUNTRY_MODIFIERS),
                )
            });
            let _marks = DOMAIN.notify($site, target(MUTATIONS[$site].1, first));
            let _inputs = crate::country_input_changes::forwarded_territory($site, first, second);
            let original: unsafe extern "system" fn(usize, usize, usize, usize) -> usize =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original(first, second, third, fourth) })
        }
    };
}
callback!(province_controller, 0);
callback!(victory_points, 1);
callback!(read_province, 2);
callback!(insert_border_conflict, 3);
callback!(erase_border_conflict, 4);
callback!(rebuild_owner_areas, 5);
callback!(initialize_provinces, 6);
#[unsafe(naked)]
unsafe extern "system-unwind" fn state_owner_entry() {
    core::arch::naked_asm!("mov rax, [rsp]", "jmp {shim}", shim = sym territory_state_owner_shim);
}
core::arch::global_asm!(
    ".text", ".globl territory_state_owner_shim", ".seh_proc territory_state_owner_shim",
    "territory_state_owner_shim:", "sub rsp, 0x38", ".seh_stackalloc 0x38", ".seh_endprologue",
    "mov [rsp + 0x20], rax", "call {body}", "add rsp, 0x38", "ret", ".seh_endproc",
    body = sym state_owner);
unsafe extern "system-unwind" {
    fn territory_state_owner_shim();
}
unsafe extern "system-unwind" fn state_owner(
    first: usize,
    second: usize,
    third: usize,
    fourth: usize,
    caller: usize,
) -> usize {
    const SITE: usize = 7;
    let timing = crate::hook_timing::enter(crate::hook_timing::Family::TerritoryChanges);
    let _occupation = crate::occupation_changes::changed_pair(
        first,
        first.wrapping_add(engine_native::cstate::COUNTRY_MODIFIERS),
    );
    let _marks = DOMAIN.notify(SITE, target(MUTATIONS[SITE].1, first));
    let _inputs = crate::country_input_changes::forwarded_territory(SITE, first, second);
    let transfer = crate::war_provenance::state_owner_before(first, third, fourth, caller);
    let original: unsafe extern "system-unwind" fn(usize, usize, usize, usize) -> usize =
        unsafe { std::mem::transmute(ORIGINAL[SITE].load(Ordering::Acquire)) };
    let result = timing.original(|| unsafe { original(first, second, third, fourth) });
    crate::war_provenance::state_owner_after(transfer);
    result
}
callback!(state_controller, 8);
callback!(reset_state, 9);
callback!(read_state, 10);
callback!(state_name_effect, 11);
callback!(recount_single_controller, 12);
callback!(refresh_cached_victory_point, 13);
callback!(add_contested_owner, 14);
callback!(remove_contested_owner, 15);

#[cfg(test)]
mod tests {
    use super::*;
    use engine_mutations::territory_mutations::StateRows;

    fn take_states(domain: &Domain) -> (Option<u64>, Vec<usize>) {
        let mut ids = Vec::new();
        let epoch = domain.take_states(&mut |id| ids.push(id));
        (epoch, ids)
    }

    fn take_provinces(domain: &Domain) -> (Option<[u64; 2]>, Vec<usize>, Vec<usize>) {
        let (mut provinces, mut states) = (Vec::new(), Vec::new());
        let epochs = domain.take_provinces(&mut |id| provinces.push(id), &mut |id| states.push(id));
        (epochs, provinces, states)
    }

    #[test]
    fn receivers_name_their_ids_through_the_live_object() {
        let mut province = vec![0_u8; 0x200];
        let mut state = vec![0_u8; 0x200];
        province[engine_native::cprovince::ID..][..4].copy_from_slice(&517_u32.to_le_bytes());
        state[engine_native::cstate::ID..][..4].copy_from_slice(&42_u32.to_le_bytes());
        let rows = StateRows {
            state: true,
            provinces: false,
        };
        assert_eq!(
            target(Receiver::Province, province.as_ptr() as usize),
            Target::Province(Some(517))
        );
        assert_eq!(
            target(Receiver::State(rows), state.as_ptr() as usize),
            Target::State {
                id: Some(42),
                state: true,
                provinces: false
            }
        );
        assert_eq!(
            target(Receiver::State(rows), province.as_ptr() as usize),
            Target::State {
                id: None,
                state: true,
                provinces: false
            }
        );
        assert_eq!(target(Receiver::Province, 0), Target::Province(None));
        assert_eq!(target(Receiver::EveryProvince, 0), Target::EveryProvince);
    }

    #[test]
    fn each_site_marks_the_sets_its_writes_reach() {
        for (site, (mutation, receiver)) in MUTATIONS.iter().enumerate() {
            let name = mutation.name;
            let domain = Domain::enabled(64, 1024);
            let states = take_states(&domain).0.unwrap();
            let provinces = take_provinces(&domain).0.unwrap();
            let target = match receiver {
                Receiver::Province => Target::Province(Some(700)),
                Receiver::State(rows) => Target::State {
                    id: Some(9),
                    state: rows.state,
                    provinces: rows.provinces,
                },
                Receiver::EveryProvince => Target::EveryProvince,
                Receiver::EveryState => Target::EveryState,
            };
            drop(domain.notify(site, target));
            let (state_epoch, state_ids) = take_states(&domain);
            let (province_epochs, province_ids, province_states) = take_provinces(&domain);
            let province_epochs = province_epochs.unwrap();
            let (state_marked, province_marked, province_state_marked) = match receiver {
                Receiver::Province => (false, true, false),
                Receiver::State(rows) => (rows.state, false, rows.provinces),
                Receiver::EveryProvince => {
                    assert_eq!(state_epoch, Some(states), "{name}");
                    assert_ne!(province_epochs[0], provinces[0], "{name}");
                    assert_eq!(province_epochs[1], provinces[1], "{name}");
                    continue;
                }
                Receiver::EveryState => {
                    assert_ne!(state_epoch, Some(states), "{name}");
                    assert_eq!(province_epochs, provinces, "{name}");
                    continue;
                }
            };
            assert_eq!(state_epoch, Some(states), "{name}");
            assert_eq!(province_epochs, provinces, "{name}");
            assert_eq!(
                state_ids,
                if state_marked { vec![9] } else { vec![] },
                "{name}"
            );
            assert_eq!(
                province_ids,
                if province_marked { vec![700] } else { vec![] },
                "{name}"
            );
            assert_eq!(
                province_states,
                if province_state_marked {
                    vec![9]
                } else {
                    vec![]
                },
                "{name}"
            );
            assert_eq!(domain.calls[site].load(Ordering::Relaxed), 1, "{name}");
        }
    }

    #[test]
    fn an_unnamed_receiver_revokes_its_sets() {
        let domain = Domain::enabled(64, 1024);
        let states = take_states(&domain).0.unwrap();
        let provinces = take_provinces(&domain).0.unwrap();
        drop(domain.notify(
            7,
            Target::State {
                id: None,
                state: true,
                provinces: true,
            },
        ));
        assert_ne!(take_states(&domain).0, Some(states));
        let after = take_provinces(&domain).0.unwrap();
        assert_eq!(after[0], provinces[0]);
        assert_ne!(after[1], provinces[1]);
        assert_eq!(domain.unnamed.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn building_writes_mark_their_location() {
        use engine_native::cprovince::buildings::{PROVINCE_FLAG, STATE_FLAG};
        let domain = Domain::enabled(64, 1024);
        let states = take_states(&domain).0.unwrap();
        let provinces = take_provinces(&domain).0.unwrap();
        drop(domain.building(PROVINCE_FLAG, 300));
        drop(domain.building(STATE_FLAG, 12));
        assert_eq!(take_states(&domain), (Some(states), vec![12]));
        assert_eq!(
            take_provinces(&domain),
            (Some(provinces), vec![300], vec![])
        );
        drop(domain.building_unnamed());
        assert_ne!(take_states(&domain).0, Some(states));
        assert_ne!(take_provinces(&domain).0.unwrap()[0], provinces[0]);
    }

    #[test]
    fn an_invalidating_boundary_revokes_every_consumer() {
        let domain = Domain::enabled(64, 1024);
        let states = take_states(&domain).0.unwrap();
        let provinces = take_provinces(&domain).0.unwrap();
        domain.boundary(false);
        assert_eq!(take_states(&domain).0, Some(states));
        assert_eq!(take_provinces(&domain).0, Some(provinces));
        domain.boundary(true);
        assert_ne!(take_states(&domain).0, Some(states));
        let after = take_provinces(&domain).0.unwrap();
        assert!(after[0] != provinces[0] && after[1] != provinces[1]);
    }

    #[test]
    fn forwarded_modifier_writers_mark_only_the_modifier_set() {
        let domain = Domain::enabled(64, 1024);
        let take = |domain: &Domain| {
            let mut ids = Vec::new();
            let epoch = domain.take_modifier_states(&mut |id| ids.push(id));
            (epoch, ids)
        };
        let states = take_states(&domain).0.unwrap();
        let provinces = take_provinces(&domain).0.unwrap();
        let modifiers = take(&domain).0.unwrap();
        drop(domain.modifier_state(Some(12)));
        assert_eq!(take(&domain), (Some(modifiers), vec![12]));
        assert_eq!(take_states(&domain), (Some(states), vec![]));
        assert_eq!(take_provinces(&domain), (Some(provinces), vec![], vec![]));
        drop(domain.modifier_state(None));
        assert_ne!(take(&domain).0, Some(modifiers));
        assert_eq!(take_states(&domain).0, Some(states));
    }

    #[test]
    fn poisoning_refuses_every_set() {
        let domain = Domain::enabled(64, 1024);
        domain.poison();
        assert!(domain.poisoned());
        assert_eq!(take_states(&domain).0, None);
        assert_eq!(take_provinces(&domain).0, None);
        assert_eq!(domain.take_modifier_states(&mut |_| ()), None);
    }
}
