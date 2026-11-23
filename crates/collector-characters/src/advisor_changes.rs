use crate::extended::characters::advisors::{Changes, Counts};
use collector_hooks::hook::Hook;
use engine_mutations::advisor_mutations::MUTATIONS;
use engine_native::Memory;
use std::{
    ops::Range,
    sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
};

const WATCHES: usize = 65536;
static WATCH: [Slot; WATCHES] = [const { Slot::new() }; WATCHES];
static EPOCH: AtomicU64 = AtomicU64::new(1);
static REVISION: AtomicU64 = AtomicU64::new(0);
static ENABLED: AtomicBool = AtomicBool::new(false);
static POISONED: AtomicBool = AtomicBool::new(false);
pub static ACTIVE: AtomicUsize = AtomicUsize::new(0);
static ORIGINAL: [AtomicUsize; 11] = [const { AtomicUsize::new(0) }; 11];
const ROLE_SITES: usize = engine_mutations::character_role_mutations::MUTATIONS.len();
static ROLE_ORIGINAL: [AtomicUsize; ROLE_SITES] = [const { AtomicUsize::new(0) }; ROLE_SITES];
static ROLES_INSTALLED: AtomicBool = AtomicBool::new(false);
static EVENTS: [AtomicU64; 11] = [const { AtomicU64::new(0) }; 11];
static COUNTS: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
static NOTIFICATION_TICKS: AtomicU64 = AtomicU64::new(0);

struct Slot {
    address: AtomicUsize,
    revision: AtomicU64,
}
impl Slot {
    const fn new() -> Self {
        Self {
            address: AtomicUsize::new(0),
            revision: AtomicU64::new(0),
        }
    }
}

fn watch(slots: &[Slot], address: usize, register: bool) -> Option<&AtomicU64> {
    if address == 0 {
        return None;
    }
    let first = ((address >> 3).wrapping_mul(0x9e3779b97f4a7c15) >> 32) & (slots.len() - 1);
    for probe in 0..slots.len().min(16) {
        let slot = &slots[(first + probe) & (slots.len() - 1)];
        let mut current = slot.address.load(Ordering::SeqCst);
        if current == 0 && register {
            current =
                match slot
                    .address
                    .compare_exchange(0, address, Ordering::SeqCst, Ordering::SeqCst)
                {
                    Ok(_) => address,
                    Err(current) => current,
                };
        }
        if current == address {
            return Some(&slot.revision);
        }
        if current == 0 {
            return None;
        }
    }
    None
}
fn increment(value: &AtomicU64) {
    if value.fetch_add(1, Ordering::SeqCst) == u64::MAX {
        POISONED.store(true, Ordering::SeqCst);
    }
}
pub fn invalidate() {
    increment(&EPOCH);
}
fn character_map(character: usize) -> usize {
    character
        .checked_add(engine_native::ccharacter::ADVISOR_MAP_HEAD)
        .unwrap_or(0)
}
fn role_map(role: usize) -> usize {
    let memory = unsafe { engine_native::memory::LiveMemory::new() };
    let owner = role
        .checked_add(engine_native::cadvisor::CHARACTER)
        .and_then(|address| engine_native::read::<u64>(&memory, address))
        .filter(|&owner| owner != 0);
    match owner {
        Some(owner) => character_map(owner as usize),
        None => {
            invalidate();
            0
        }
    }
}

struct Mutation {
    revision: Option<&'static AtomicU64>,
    ticks: u64,
    _character: Option<collector_certificates::watch_registry::Notification<'static>>,
    _census: Option<collector_certificates::watch_registry::Notification<'static>>,
}

fn census_site(site: usize) -> Option<collector_observers::census_changes::Site> {
    use collector_observers::census_changes::Site;
    match MUTATIONS[site].name {
        "insert_map" => Some(Site::AdvisorMapInsert),
        "insert_map_value" => Some(Site::AdvisorMapMove),
        "erase_map" => Some(Site::AdvisorMapErase),
        "merge_character" => Some(Site::Merge),
        "read_character" => Some(Site::Parse),
        "destroy_character" => Some(Site::Destroy),
        _ => None,
    }
}
impl Mutation {
    fn enter(address: impl FnOnce() -> usize, site: usize) -> Self {
        ACTIVE.fetch_add(1, Ordering::SeqCst);
        let begin = collector_primitives::clock::qpc();
        increment(&REVISION);
        let address = address();
        let character = cfg!(feature = "character_changes")
            .then(|| collector_observers::character_changes::advisor_map(address));
        let revision = watch(&WATCH, address, false);
        if let Some(revision) = revision {
            increment(revision);
        }
        EVENTS[site].fetch_add(1, Ordering::Relaxed);
        let census = census_site(site).and_then(collector_observers::census_changes::forwarded);
        Self {
            revision,
            ticks: collector_primitives::clock::qpc()
                .saturating_sub(begin)
                .max(0) as u64,
            _character: character,
            _census: census,
        }
    }
}
impl Drop for Mutation {
    fn drop(&mut self) {
        let begin = collector_primitives::clock::qpc();
        if let Some(revision) = self.revision {
            increment(revision);
        }
        increment(&REVISION);
        NOTIFICATION_TICKS.fetch_add(
            self.ticks
                + collector_primitives::clock::qpc()
                    .saturating_sub(begin)
                    .max(0) as u64,
            Ordering::Relaxed,
        );
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct Observer;
impl Changes for Observer {
    fn quiet(&self) -> Option<(u64, u64)> {
        if !ENABLED.load(Ordering::SeqCst) || POISONED.load(Ordering::SeqCst) {
            return None;
        }
        let value = (
            EPOCH.load(Ordering::SeqCst),
            REVISION.load(Ordering::SeqCst),
        );
        (ACTIVE.load(Ordering::SeqCst) == 0).then_some(value)
    }
    fn revision(&self, map: usize) -> Option<(u64, u64)> {
        let (epoch, _) = self.quiet()?;
        let revision = watch(&WATCH, map, true)?.load(Ordering::SeqCst);
        (ACTIVE.load(Ordering::SeqCst) == 0).then_some((epoch, revision))
    }
}

pub fn record(counts: Counts) {
    for (target, value) in
        COUNTS
            .iter()
            .zip([counts.reused, counts.read, counts.roles, counts.planned])
    {
        target.fetch_add(value, Ordering::Relaxed);
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
        if cfg!(feature = "advisor_changes") {
            let callbacks = [
                construct as *const () as usize,
                destroy as *const () as usize,
                set_idea as *const () as usize,
                read_role as *const () as usize,
                insert_map as *const () as usize,
                insert_map_value as *const () as usize,
                erase_map as *const () as usize,
                clear_map as *const () as usize,
                merge_character as *const () as usize,
                read_character as *const () as usize,
                destroy_character as *const () as usize,
            ];
            for (index, site) in MUTATIONS.iter().enumerate() {
                let (at, original) = collector_hooks::located_hook(base, site.rva)?;
                let hook = unsafe { Hook::new(at, callbacks[index], original) }
                    .map_err(|_| "native hook construction failed")?;
                ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                hooks.push(hook);
            }
            let roles = engine_mutations::character_role_mutations::MUTATIONS;
            let mut located = [(0, [0; 16]); ROLE_SITES];
            if cfg!(feature = "character_changes")
                && roles.iter().zip(&mut located).all(|(site, slot)| {
                    collector_hooks::located_hook(base, site.rva)
                        .map(|code| *slot = code)
                        .is_ok()
                })
            {
                let callbacks: [usize; ROLE_SITES] = [
                    country_leader_modifiers as *const () as usize,
                    country_leader_destroy as *const () as usize,
                    country_leader_map_insert as *const () as usize,
                    country_leader_create as *const () as usize,
                    advisor_modifiers as *const () as usize,
                    country_leader_ideology as *const () as usize,
                    character_name as *const () as usize,
                    character_repair as *const () as usize,
                ];
                for (index, &(at, original)) in located.iter().enumerate() {
                    let hook = unsafe { Hook::new(at, callbacks[index], original) }
                        .map_err(|_| "native hook construction failed")?;
                    ROLE_ORIGINAL[index].store(hook.trampoline(), Ordering::Release);
                    hooks.push(hook);
                }
                ROLES_INSTALLED.store(true, Ordering::SeqCst);
            }
        }
        Ok(Self { hooks })
    }
    pub fn set_enabled(&mut self, enabled: bool) -> Result<bool, &'static str> {
        if !enabled {
            ENABLED.store(false, Ordering::SeqCst);
            collector_observers::character_changes::set_enabled(false);
            collector_observers::roster_changes::set_repair_forward(false);
            collector_observers::country_input_changes::forwarder(
                collector_observers::country_input_changes::Forwarder::Roles,
                false,
            );
            collector_observers::census_changes::forwarder(
                collector_observers::census_changes::Forwarder::Advisors,
                false,
            );
        }
        if !Hook::set_family_enabled_shared(&mut self.hooks, enabled)
            .map_err(|_| "native hook state change failed")?
        {
            return Ok(false);
        }
        ENABLED.store(enabled && !self.hooks.is_empty(), Ordering::SeqCst);
        collector_observers::character_changes::set_enabled(
            enabled && !self.hooks.is_empty() && ROLES_INSTALLED.load(Ordering::SeqCst),
        );
        collector_observers::roster_changes::set_repair_forward(
            enabled && !self.hooks.is_empty() && ROLES_INSTALLED.load(Ordering::SeqCst),
        );
        collector_observers::country_input_changes::forwarder(
            collector_observers::country_input_changes::Forwarder::Roles,
            enabled && !self.hooks.is_empty() && ROLES_INSTALLED.load(Ordering::SeqCst),
        );
        collector_observers::census_changes::forwarder(
            collector_observers::census_changes::Forwarder::Advisors,
            enabled && !self.hooks.is_empty() && ROLES_INSTALLED.load(Ordering::SeqCst),
        );
        Ok(true)
    }
    pub fn executable_ranges(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.hooks.iter().flat_map(Hook::executable_ranges)
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        // SAFETY: like each Hook's relay, the character watch tables are
        // dropped only once the hooks were never enabled, or were removed and
        // drained after the last capture.
        unsafe { collector_observers::character_changes::release() };
    }
}

macro_rules! callback {
    ($name:ident, $site:expr, ($($arg:ident:$ty:ty),*) -> $ret:ty, $map:expr) => {
        unsafe extern "system" fn $name($($arg:$ty),*) -> $ret {
            let timing = collector_timing::hook_timing::enter(collector_timing::hook_timing::Family::AdvisorChanges);
            let _mutation = Mutation::enter(|| $map, $site);
            let original: unsafe extern "system" fn($($ty),*) -> $ret =
                unsafe { std::mem::transmute(ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original($($arg),*) })
        }
    };
}
callback!(construct, 0, (this:usize, owner:usize) -> usize, character_map(owner));
callback!(destroy, 1, (this:usize) -> (), role_map(this));
callback!(set_idea, 2, (this:usize, token:usize) -> (), role_map(this));
callback!(read_role, 3, (this:usize, reader:usize, token:i32) -> (), role_map(this));
callback!(insert_map, 4, (this:usize, result:usize, key:usize, value:usize) -> usize, this);
callback!(insert_map_value, 5, (this:usize, result:usize, key:usize, value:usize) -> usize, this);
callback!(erase_map, 6, (this:usize, range:usize) -> usize, this);
callback!(clear_map, 7, (this:usize, allocator:usize, node:usize) -> (), this);
callback!(merge_character, 8, (this:usize, source:usize, country:usize) -> (), character_map(this));
callback!(read_character, 9, (this:usize, reader:usize, token:i32) -> (), character_map(this));
callback!(destroy_character, 10, (this:usize) -> (), character_map(this));

macro_rules! role_callback {
    ($name:ident, $site:expr, ($($arg:ident:$ty:ty),*), $character:expr) => {
        unsafe extern "system" fn $name($($arg:$ty),*) -> usize {
            let timing = collector_timing::hook_timing::enter(collector_timing::hook_timing::Family::AdvisorChanges);
            let character = $character;
            let _notification = collector_observers::character_changes::notify(character, $site);
            let _inputs = collector_observers::country_input_changes::forwarded_role($site, character);
            let _census = ($site == roles::COUNTRY_LEADER_MAP_INSERT)
                .then(|| collector_observers::census_changes::forwarded(collector_observers::census_changes::Site::LeaderMapInsert))
                .flatten();
            let original: unsafe extern "system" fn($($ty),*) -> usize =
                unsafe { std::mem::transmute(ROLE_ORIGINAL[$site].load(Ordering::Acquire)) };
            timing.original(|| unsafe { original($($arg),*) })
        }
    };
}
use engine_mutations::character_role_mutations as roles;
role_callback!(country_leader_modifiers, roles::COUNTRY_LEADER_MODIFIERS, (leader: usize),
    collector_observers::character_changes::role_character(leader, engine_native_politics::ccountry_leader::CHARACTER));
role_callback!(country_leader_destroy, roles::COUNTRY_LEADER_DESTROY, (leader: usize),
    collector_observers::character_changes::role_character(leader, engine_native_politics::ccountry_leader::CHARACTER));
role_callback!(country_leader_map_insert, roles::COUNTRY_LEADER_MAP_INSERT,
    (map: usize, result: usize, key: usize, value: usize),
    Some(map.saturating_sub(engine_native::ccharacter::LEADER_MAP_HEAD)));
role_callback!(country_leader_create, roles::COUNTRY_LEADER_CREATE,
    (out: usize, character: usize, definition: usize), Some(character));
role_callback!(advisor_modifiers, roles::ADVISOR_MODIFIERS, (advisor: usize),
    collector_observers::character_changes::role_character(advisor, engine_native::cadvisor::CHARACTER));
role_callback!(character_name, roles::CHARACTER_NAME, (character: usize, name: usize),
    Some(character));
unsafe extern "system" fn character_repair(character: usize) -> usize {
    let timing =
        collector_timing::hook_timing::enter(collector_timing::hook_timing::Family::AdvisorChanges);
    let _roster = collector_observers::roster_changes::character_repaired();
    let _notification =
        collector_observers::character_changes::notify(Some(character), roles::CHARACTER_REPAIR);
    let _inputs = collector_observers::country_input_changes::forwarded_role(
        roles::CHARACTER_REPAIR,
        Some(character),
    );
    let original: unsafe extern "system" fn(usize) -> usize = unsafe {
        std::mem::transmute(ROLE_ORIGINAL[roles::CHARACTER_REPAIR].load(Ordering::Acquire))
    };
    timing.original(|| unsafe { original(character) })
}
role_callback!(country_leader_ideology, roles::COUNTRY_LEADER_IDEOLOGY,
    (effect: usize, scope: usize), None);

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_registry_never_certifies_colliding_or_unregistered_maps() {
        let slots = [const { Slot::new() }; 4];
        for address in [8, 16, 24, 32] {
            assert!(watch(&slots, address, false).is_none());
            watch(&slots, address, true)
                .unwrap()
                .store(address as u64, Ordering::SeqCst);
        }
        assert!(watch(&slots, 40, true).is_none());
        assert!(watch(&slots, 0, true).is_none());
        for address in [8, 16, 24, 32] {
            assert_eq!(
                watch(&slots, address, false)
                    .unwrap()
                    .load(Ordering::SeqCst),
                address as u64
            );
        }
    }
}
